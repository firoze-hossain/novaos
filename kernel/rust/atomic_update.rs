//! kernel/rust/atomic_update.rs - Month 6: Atomic OS Update Mechanism
//!
//! Provides a safe, atomic A/B boot slot update and rollback engine.
//! Prevents power-loss corruption by combining:
//! 1. Dual Boot Slots (Slot A: primary, Slot B: secondary).
//! 2. Package Signing: Cryptographic signature verification (HMAC-SHA256)
//!    using `pkgsign_core` before any update binary is staged.
//! 3. Write-Ahead Journaling: Multi-file transactional writes via
//!    `journal.rs` so that writing the new kernel binary and updating
//!    the active boot slot are committed atomically to disk.
//! 4. Bootloader & Kernel Hooks: Automatic boot retry countdown with
//!    failsafe rollback to the last-known-good slot if an update fails
//!    to boot or pass health checks.

use crate::pkgsign_core::compute_signature;
use crate::sha256::Sha256Streaming;
use crate::spinlock::SpinLock;

pub const BOOTSLOT_MAGIC: [u8; 8] = *b"NOVABOOT";
pub const BOOTSLOT_FILENAME: &[u8] = b"BOOTSLOT.DAT\0";
pub const SLOT_A_FILENAME: &[u8] = b"NOVAOS.BIN\0";
pub const SLOT_B_FILENAME: &[u8] = b"NOVAOS_B.BIN\0";

pub const NOVA_BOOT_SLOT_A: u8 = 0;
pub const NOVA_BOOT_SLOT_B: u8 = 1;

pub const NOVA_SLOT_STATE_NORMAL: u8 = 0;
pub const NOVA_SLOT_STATE_TESTING: u8 = 1;
pub const NOVA_SLOT_STATE_ROLLED_BACK: u8 = 2;

pub const DEFAULT_MAX_BOOT_ATTEMPTS: u8 = 3;

pub const NOVA_UPDATE_OK: i32 = 0;
pub const NOVA_UPDATE_ERR_INVALID_OP: i32 = -1;
pub const NOVA_UPDATE_ERR_BAD_HEADER: i32 = -2;
pub const NOVA_UPDATE_ERR_BAD_SIG: i32 = -3;
pub const NOVA_UPDATE_ERR_IO: i32 = -4;
pub const NOVA_UPDATE_ERR_JOURNAL: i32 = -5;
pub const NOVA_UPDATE_ERR_PERM: i32 = -6;
pub const NOVA_UPDATE_ERR_FAULT: i32 = -7;

extern "C" {
    fn vfs_read_file(filename: *const u8, buf: *mut u8, buf_size: u32) -> i32;
    fn vfs_write_file(filename: *const u8, data: *const u8, size: u32) -> bool;
    fn vfs_delete_file(filename: *const u8) -> bool;
    fn rust_journal_begin();
    fn rust_journal_commit() -> bool;
}

/// Constant-time byte slice comparison to defend against timing side-channels.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Adler-32 checksum calculation over fixed configuration records.
fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Persistent on-disk boot slot configuration (128 bytes, packed).
#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct BootSlotConfig {
    pub magic: [u8; 8],            // "NOVABOOT"
    pub struct_version: u32,       // 1
    pub active_slot: u8,           // 0 = Slot A, 1 = Slot B
    pub fallback_slot: u8,         // 0 = Slot A, 1 = Slot B
    pub slot_state: u8,            // 0 = NORMAL, 1 = TESTING, 2 = ROLLED_BACK
    pub boot_attempts: u8,         // Remaining attempts before auto-rollback
    pub slot_a_version: [u8; 16],  // e.g. "0.88.0\0"
    pub slot_b_version: [u8; 16],  // e.g. "0.89.0\0"
    pub slot_a_sha256: [u8; 32],   // SHA-256 digest of Slot A kernel
    pub slot_b_sha256: [u8; 32],   // SHA-256 digest of Slot B kernel
    pub reserved: [u8; 20],        // Padding to 124 bytes
    pub checksum: u32,             // Adler-32 over preceding 124 bytes
}

impl BootSlotConfig {
    pub fn default_new() -> Self {
        let mut cfg = BootSlotConfig {
            magic: BOOTSLOT_MAGIC,
            struct_version: 1,
            active_slot: NOVA_BOOT_SLOT_A,
            fallback_slot: NOVA_BOOT_SLOT_A,
            slot_state: NOVA_SLOT_STATE_NORMAL,
            boot_attempts: DEFAULT_MAX_BOOT_ATTEMPTS,
            slot_a_version: [0u8; 16],
            slot_b_version: [0u8; 16],
            slot_a_sha256: [0u8; 32],
            slot_b_sha256: [0u8; 32],
            reserved: [0u8; 20],
            checksum: 0,
        };
        let v_a = b"0.88.0\0";
        cfg.slot_a_version[..v_a.len()].copy_from_slice(v_a);
        let v_b = b"empty\0";
        cfg.slot_b_version[..v_b.len()].copy_from_slice(v_b);
        cfg.update_checksum();
        cfg
    }

    pub fn update_checksum(&mut self) {
        let raw = unsafe {
            let ptr = self as *const Self as *const u8;
            core::slice::from_raw_parts(ptr, 124)
        };
        self.checksum = adler32(raw);
    }

    pub fn is_valid(&self) -> bool {
        if self.magic != BOOTSLOT_MAGIC || self.struct_version != 1 {
            return false;
        }
        let raw = unsafe {
            let ptr = self as *const Self as *const u8;
            core::slice::from_raw_parts(ptr, 124)
        };
        adler32(raw) == self.checksum
    }
}

/// Userland status query structure (mirrors nova_update_status_t in atomic_update.h).
#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct NovaUpdateStatus {
    pub active_slot: u8,
    pub fallback_slot: u8,
    pub slot_state: u8,
    pub boot_attempts: u8,
    pub slot_a_version: [u8; 16],
    pub slot_b_version: [u8; 16],
    pub last_error: i32,
}

/// In-kernel state guarded by spinlock.
pub struct OsUpdateState {
    pub config: BootSlotConfig,
    pub initialized: bool,
    pub last_error: i32,
}

impl OsUpdateState {
    pub const fn new() -> Self {
        OsUpdateState {
            config: BootSlotConfig {
                magic: [0u8; 8],
                struct_version: 0,
                active_slot: 0,
                fallback_slot: 0,
                slot_state: 0,
                boot_attempts: 0,
                slot_a_version: [0u8; 16],
                slot_b_version: [0u8; 16],
                slot_a_sha256: [0u8; 32],
                slot_b_sha256: [0u8; 32],
                reserved: [0u8; 20],
                checksum: 0,
            },
            initialized: false,
            last_error: 0,
        }
    }
}

static UPDATE_STATE: SpinLock<OsUpdateState> = SpinLock::new(OsUpdateState::new());

/// Ensures the boot slot configuration is loaded into memory.
fn ensure_config_loaded(state: &mut OsUpdateState) {
    if state.initialized {
        return;
    }

    let mut buf = [0u8; 128];
    let n = unsafe { vfs_read_file(BOOTSLOT_FILENAME.as_ptr(), buf.as_mut_ptr(), 128) };
    if n == 128 {
        let loaded = unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const BootSlotConfig) };
        if loaded.is_valid() {
            state.config = loaded;
            state.initialized = true;
            return;
        }
    }

    // Initialize clean default configuration
    let mut default_cfg = BootSlotConfig::default_new();
    default_cfg.update_checksum();
    unsafe {
        let ptr = &default_cfg as *const BootSlotConfig as *const u8;
        vfs_delete_file(BOOTSLOT_FILENAME.as_ptr());
        vfs_write_file(BOOTSLOT_FILENAME.as_ptr(), ptr, 128);
    }
    state.config = default_cfg;
    state.initialized = true;
}

/// Writes the current in-memory config to disk inside a journaled transaction.
fn save_config_locked(state: &mut OsUpdateState) -> bool {
    state.config.update_checksum();
    let ptr = &state.config as *const BootSlotConfig as *const u8;
    unsafe {
        rust_journal_begin();
        vfs_delete_file(BOOTSLOT_FILENAME.as_ptr());
        let ok = vfs_write_file(BOOTSLOT_FILENAME.as_ptr(), ptr, 128);
        let commit_ok = rust_journal_commit();
        ok && commit_ok
    }
}

/// Parses and verifies an OS update package.
/// The package layout follows pkg_header_t (128 bytes) followed by payload:
/// - magic: "NVPK" or "NVUP" (offset 0, 4 bytes)
/// - name: (offset 4, 16 bytes)
/// - version: (offset 20, 8 bytes)
/// - description: (offset 28, 64 bytes)
/// - payload_size: (offset 92, 4 bytes)
/// - signature: (offset 96, 32 bytes)
/// - payload bytes: offset 128..128+payload_size
fn verify_update_package(pkg: &[u8]) -> Result<(u32, [u8; 16], [u8; 32], &[u8]), i32> {
    if pkg.len() < 128 {
        return Err(NOVA_UPDATE_ERR_BAD_HEADER);
    }

    let magic = &pkg[0..4];
    if magic != b"NVPK" && magic != b"NVUP" {
        return Err(NOVA_UPDATE_ERR_BAD_HEADER);
    }

    let payload_size = u32::from_le_bytes([pkg[92], pkg[93], pkg[94], pkg[95]]);
    if (128 + payload_size as usize) != pkg.len() {
        return Err(NOVA_UPDATE_ERR_BAD_HEADER);
    }

    let payload = &pkg[128..128 + payload_size as usize];

    // Compute cryptographic signature with signature field zeroed
    let mut header_zeroed = [0u8; 128];
    header_zeroed.copy_from_slice(&pkg[..128]);
    for b in &mut header_zeroed[96..128] {
        *b = 0;
    }

    let expected_sig = compute_signature(&header_zeroed, payload);
    let actual_sig = &pkg[96..128];
    if !constant_time_eq(&expected_sig, actual_sig) {
        return Err(NOVA_UPDATE_ERR_BAD_SIG);
    }

    // Extract version string (up to 15 chars + NUL)
    let mut version = [0u8; 16];
    let v_slice = &pkg[20..28];
    for (i, &b) in v_slice.iter().enumerate() {
        if b == 0 {
            break;
        }
        version[i] = b;
    }

    // Compute SHA-256 of payload
    let mut hasher = Sha256Streaming::new();
    hasher.update(payload);
    let payload_sha256 = hasher.finalize();

    Ok((payload_size, version, payload_sha256, payload))
}

// -----------------------------------------------------------------------------
// Exported C Interface
// -----------------------------------------------------------------------------

/// Initializes the atomic update subsystem.
#[no_mangle]
pub extern "C" fn rust_os_update_init() {
    let mut state = UPDATE_STATE.lock();
    ensure_config_loaded(&mut state);
}

/// Early boot hook: validates active slot and executes auto-rollback if boot attempts expired.
#[no_mangle]
pub extern "C" fn rust_os_update_boot_hook() {
    let mut state = UPDATE_STATE.lock();
    ensure_config_loaded(&mut state);

    if state.config.slot_state == NOVA_SLOT_STATE_TESTING {
        if state.config.boot_attempts > 1 {
            // Decrement remaining boot attempt and save
            state.config.boot_attempts -= 1;
            save_config_locked(&mut state);
        } else {
            // Boot attempts exhausted without confirmation -> automatic rollback!
            state.config.active_slot = state.config.fallback_slot;
            state.config.slot_state = NOVA_SLOT_STATE_ROLLED_BACK;
            state.config.boot_attempts = DEFAULT_MAX_BOOT_ATTEMPTS;
            save_config_locked(&mut state);
        }
    }
}

/// Confirms the currently booted slot as healthy (commits update permanently).
#[no_mangle]
pub extern "C" fn rust_os_update_confirm_healthy() -> i32 {
    let mut state = UPDATE_STATE.lock();
    ensure_config_loaded(&mut state);

    if state.config.slot_state == NOVA_SLOT_STATE_TESTING {
        state.config.slot_state = NOVA_SLOT_STATE_NORMAL;
        state.config.boot_attempts = DEFAULT_MAX_BOOT_ATTEMPTS;
        if save_config_locked(&mut state) {
            NOVA_UPDATE_OK
        } else {
            NOVA_UPDATE_ERR_IO
        }
    } else {
        NOVA_UPDATE_OK
    }
}

/// Staged apply of a verified, signed OS update package to the secondary boot slot.
#[no_mangle]
pub extern "C" fn rust_os_update_apply_package(pkg_ptr: *const u8, pkg_len: u32) -> i32 {
    if pkg_ptr.is_null() || pkg_len < 128 {
        return NOVA_UPDATE_ERR_FAULT;
    }

    let pkg = unsafe { core::slice::from_raw_parts(pkg_ptr, pkg_len as usize) };
    let (payload_size, version, payload_sha256, payload) = match verify_update_package(pkg) {
        Ok(res) => res,
        Err(err) => {
            let mut state = UPDATE_STATE.lock();
            state.last_error = err;
            return err;
        }
    };

    let mut state = UPDATE_STATE.lock();
    ensure_config_loaded(&mut state);

    // Identify target secondary slot (opposite of current active)
    let current_slot = state.config.active_slot;
    let target_slot = if current_slot == NOVA_BOOT_SLOT_A {
        NOVA_BOOT_SLOT_B
    } else {
        NOVA_BOOT_SLOT_A
    };

    let target_filename = if target_slot == NOVA_BOOT_SLOT_B {
        SLOT_B_FILENAME
    } else {
        SLOT_A_FILENAME
    };

    // Execute atomic update transaction:
    // 1. Write target binary to secondary slot
    // 2. Update BOOTSLOT.DAT to switch active slot to target with state TESTING
    // 3. Commit journal transaction
    unsafe {
        rust_journal_begin();

        vfs_delete_file(target_filename.as_ptr());
        let ok_bin = vfs_write_file(
            target_filename.as_ptr(),
            payload.as_ptr(),
            payload_size,
        );
        if !ok_bin {
            rust_journal_commit();
            state.last_error = NOVA_UPDATE_ERR_IO;
            return NOVA_UPDATE_ERR_IO;
        }

        state.config.fallback_slot = current_slot;
        state.config.active_slot = target_slot;
        state.config.slot_state = NOVA_SLOT_STATE_TESTING;
        state.config.boot_attempts = DEFAULT_MAX_BOOT_ATTEMPTS;

        if target_slot == NOVA_BOOT_SLOT_B {
            state.config.slot_b_version = version;
            state.config.slot_b_sha256 = payload_sha256;
        } else {
            state.config.slot_a_version = version;
            state.config.slot_a_sha256 = payload_sha256;
        }

        state.config.update_checksum();
        let cfg_ptr = &state.config as *const BootSlotConfig as *const u8;
        vfs_delete_file(BOOTSLOT_FILENAME.as_ptr());
        let ok_cfg = vfs_write_file(BOOTSLOT_FILENAME.as_ptr(), cfg_ptr, 128);

        let commit_ok = rust_journal_commit();
        if !ok_cfg || !commit_ok {
            state.last_error = NOVA_UPDATE_ERR_JOURNAL;
            return NOVA_UPDATE_ERR_JOURNAL;
        }
    }

    state.last_error = NOVA_UPDATE_OK;
    NOVA_UPDATE_OK
}

/// Manually triggers a rollback to the fallback boot slot.
#[no_mangle]
pub extern "C" fn rust_os_update_rollback() -> i32 {
    let mut state = UPDATE_STATE.lock();
    ensure_config_loaded(&mut state);

    state.config.active_slot = state.config.fallback_slot;
    state.config.slot_state = NOVA_SLOT_STATE_ROLLED_BACK;
    state.config.boot_attempts = DEFAULT_MAX_BOOT_ATTEMPTS;

    if save_config_locked(&mut state) {
        state.last_error = NOVA_UPDATE_OK;
        NOVA_UPDATE_OK
    } else {
        state.last_error = NOVA_UPDATE_ERR_IO;
        NOVA_UPDATE_ERR_IO
    }
}

/// Retrieves the current update status for userland/admin query.
#[no_mangle]
pub extern "C" fn rust_os_update_get_status(out_status: *mut NovaUpdateStatus) -> i32 {
    if out_status.is_null() {
        return NOVA_UPDATE_ERR_FAULT;
    }

    let mut state = UPDATE_STATE.lock();
    ensure_config_loaded(&mut state);

    let status = NovaUpdateStatus {
        active_slot: state.config.active_slot,
        fallback_slot: state.config.fallback_slot,
        slot_state: state.config.slot_state,
        boot_attempts: state.config.boot_attempts,
        slot_a_version: state.config.slot_a_version,
        slot_b_version: state.config.slot_b_version,
        last_error: state.last_error,
    };

    unsafe {
        core::ptr::write_unaligned(out_status, status);
    }
    NOVA_UPDATE_OK
}

/// In-kernel self-test verifying safe apply, package verification, rollback, and state transitions.
#[no_mangle]
pub extern "C" fn rust_os_update_selftest() -> i32 {
    let mut code = 0i32;

    // Test 1: Config serialization, Adler-32 checksum generation & validation
    let mut cfg = BootSlotConfig::default_new();
    if !cfg.is_valid() {
        code |= 1 << 0;
    }
    cfg.boot_attempts = 1;
    if cfg.is_valid() {
        // Must be invalid until checksum is recalculated
        code |= 1 << 1;
    }
    cfg.update_checksum();
    if !cfg.is_valid() {
        code |= 1 << 2;
    }

    // Test 2: Synthesize and sign an authentic OS update package
    let mut pkg = [0u8; 128 + 64];
    // Magic: "NVUP"
    pkg[0] = b'N';
    pkg[1] = b'V';
    pkg[2] = b'U';
    pkg[3] = b'P';
    // Name: "OSUPDATE"
    let name = b"OSUPDATE";
    pkg[4..4 + name.len()].copy_from_slice(name);
    // Version: "0.89.0"
    let ver = b"0.89.0";
    pkg[20..20 + ver.len()].copy_from_slice(ver);
    // Payload size: 64 bytes
    let payload_size = 64u32;
    pkg[92..96].copy_from_slice(&payload_size.to_le_bytes());
    // Payload
    let payload = b"NovaOS Verified Test Kernel Image Update Payload v0.89.0 (64 bytes)";
    pkg[128..128 + 64].copy_from_slice(&payload[..64]);

    // Sign package: HMAC-SHA256 over header (signature zeroed) + payload
    let sig = compute_signature(&pkg[..128], &pkg[128..128 + 64]);
    pkg[96..128].copy_from_slice(&sig);

    // Valid package verification
    match verify_update_package(&pkg) {
        Ok((sz, v, hash, pl)) => {
            if sz != 64 || &v[..6] != b"0.89.0" || pl.len() != 64 {
                code |= 1 << 3;
            }
        }
        Err(_) => code |= 1 << 4,
    }

    // Test 3: Tampered payload rejection
    let mut tampered_pkg = pkg;
    tampered_pkg[128] ^= 0xFF; // Flip bit in payload
    if verify_update_package(&tampered_pkg).is_ok() {
        code |= 1 << 5;
    }

    // Test 4: Tampered version/manifest header rejection
    let mut tampered_hdr = pkg;
    tampered_hdr[20] = b'9'; // Modify version from 0.89.0 to 9.89.0
    if verify_update_package(&tampered_hdr).is_ok() {
        code |= 1 << 6;
    }

    // Test 5: Invalid magic rejection
    let mut bad_magic = pkg;
    bad_magic[0] = b'X';
    if verify_update_package(&bad_magic).is_ok() {
        code |= 1 << 7;
    }

    // Test 6: Safe apply to secondary slot with transactional commit
    let apply_res = rust_os_update_apply_package(pkg.as_ptr(), pkg.len() as u32);
    if apply_res != NOVA_UPDATE_OK {
        code |= 1 << 8;
    } else {
        let mut status = NovaUpdateStatus {
            active_slot: 0,
            fallback_slot: 0,
            slot_state: 0,
            boot_attempts: 0,
            slot_a_version: [0u8; 16],
            slot_b_version: [0u8; 16],
            last_error: 0,
        };
        rust_os_update_get_status(&mut status);
        if status.active_slot != NOVA_BOOT_SLOT_B ||
           status.fallback_slot != NOVA_BOOT_SLOT_A ||
           status.slot_state != NOVA_SLOT_STATE_TESTING ||
           status.boot_attempts != DEFAULT_MAX_BOOT_ATTEMPTS {
            code |= 1 << 9;
        }
    }

    // Test 7: Boot retry decrement and simulated boot attempts
    rust_os_update_boot_hook(); // attempts: 3 -> 2
    rust_os_update_boot_hook(); // attempts: 2 -> 1
    {
        let mut status = NovaUpdateStatus {
            active_slot: 0,
            fallback_slot: 0,
            slot_state: 0,
            boot_attempts: 0,
            slot_a_version: [0u8; 16],
            slot_b_version: [0u8; 16],
            last_error: 0,
        };
        rust_os_update_get_status(&mut status);
        if status.boot_attempts != 1 || status.slot_state != NOVA_SLOT_STATE_TESTING {
            code |= 1 << 10;
        }
    }

    // Test 8: Auto-rollback on retry exhaustion
    rust_os_update_boot_hook(); // attempts 1 -> 0 -> triggers auto-rollback to Slot A!
    {
        let mut status = NovaUpdateStatus {
            active_slot: 0,
            fallback_slot: 0,
            slot_state: 0,
            boot_attempts: 0,
            slot_a_version: [0u8; 16],
            slot_b_version: [0u8; 16],
            last_error: 0,
        };
        rust_os_update_get_status(&mut status);
        if status.active_slot != NOVA_BOOT_SLOT_A || status.slot_state != NOVA_SLOT_STATE_ROLLED_BACK {
            code |= 1 << 11;
        }
    }

    // Test 9: Health confirmation commits slot
    // Re-apply to Slot B and confirm healthy
    let _ = rust_os_update_apply_package(pkg.as_ptr(), pkg.len() as u32);
    let confirm_res = rust_os_update_confirm_healthy();
    if confirm_res != NOVA_UPDATE_OK {
        code |= 1 << 12;
    } else {
        let mut status = NovaUpdateStatus {
            active_slot: 0,
            fallback_slot: 0,
            slot_state: 0,
            boot_attempts: 0,
            slot_a_version: [0u8; 16],
            slot_b_version: [0u8; 16],
            last_error: 0,
        };
        rust_os_update_get_status(&mut status);
        if status.active_slot != NOVA_BOOT_SLOT_B || status.slot_state != NOVA_SLOT_STATE_NORMAL {
            code |= 1 << 13;
        }
    }

    // Test 10: Manual rollback reverts to Slot A
    let rollback_res = rust_os_update_rollback();
    if rollback_res != NOVA_UPDATE_OK {
        code |= 1 << 14;
    } else {
        let mut status = NovaUpdateStatus {
            active_slot: 0,
            fallback_slot: 0,
            slot_state: 0,
            boot_attempts: 0,
            slot_a_version: [0u8; 16],
            slot_b_version: [0u8; 16],
            last_error: 0,
        };
        rust_os_update_get_status(&mut status);
        if status.active_slot != NOVA_BOOT_SLOT_A {
            code |= 1 << 15;
        }
        // Restore clean normal state on Slot A
        {
            let mut state = UPDATE_STATE.lock();
            state.config.active_slot = NOVA_BOOT_SLOT_A;
            state.config.fallback_slot = NOVA_BOOT_SLOT_B;
            state.config.slot_state = NOVA_SLOT_STATE_NORMAL;
            state.config.boot_attempts = DEFAULT_MAX_BOOT_ATTEMPTS;
            save_config_locked(&mut state);
        }
    }

    code
}
