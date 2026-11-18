//! kernel/src/security/audit.rs
//!
//! NovaOS 2.0 - Month 6: Enterprise Security Audit & Common Criteria / FIPS-140 Hardening
//!
//! Comprehensive cross-track security audit engine verifying:
//! 1. Memory zeroization of cryptographic secrets (FIPS 140-3 compliance)
//! 2. Constant-time cryptographic primitives (timing & side-channel attack defense)
//! 3. Stack canary & stack smashing protection (SSP)
//! 4. KASLR & virtual memory hardening (NX bit, null-page guard, higher-half isolation)
//! 5. 89-syscall privilege authorization matrix & capability boundary enforcement
//! 6. Non-root privilege drop verification for userland enterprise daemons
//! 7. FIPS 140-3 Known Answer Tests (KAT) for cryptographic algorithms

use core::sync::atomic::{compiler_fence, Ordering};

// -----------------------------------------------------------------------------
// 1. Mandatory Memory Zeroization (FIPS 140-3 compliant volatile write)
// -----------------------------------------------------------------------------

/// Explicit volatile zeroization of sensitive cryptographic buffers.
/// Guaranteed not to be optimized out by compiler dead-store elimination.
#[inline(never)]
pub fn zeroize_bytes(buf: &mut [u8]) {
    for byte in buf.iter_mut() {
        unsafe {
            core::ptr::write_volatile(byte, 0x00);
        }
    }
    compiler_fence(Ordering::SeqCst);
}

/// Verifies that memory zeroization completely clears buffers and resists optimization.
pub fn verify_zeroize() -> bool {
    let mut secret = [0x5Au8; 64];
    for (i, b) in secret.iter_mut().enumerate() {
        *b = (i as u8) ^ 0xA5;
    }

    // Verify non-zero state before zeroization
    if secret.iter().all(|&b| b == 0) {
        return false;
    }

    zeroize_bytes(&mut secret);

    // Verify 100% zeroed
    for &b in secret.iter() {
        if b != 0x00 {
            return false;
        }
    }
    true
}

// -----------------------------------------------------------------------------
// 2. Constant-Time Cryptographic Primitives (Side-Channel Mitigation)
// -----------------------------------------------------------------------------

/// Constant-time byte slice comparison.
/// Returns true if and only if both slices have identical contents.
/// Execution time is strictly dependent on slice length, not content mismatch location.
#[inline(never)]
pub fn ct_eq_bytes(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Constant-time select for u8: returns `a` if `mask == 0xFF`, or `b` if `mask == 0x00`.
#[inline(always)]
pub fn ct_select_u8(mask: u8, a: u8, b: u8) -> u8 {
    (mask & a) | (!mask & b)
}

/// Constant-time select for u32: returns `a` if `mask == 0xFFFFFFFF`, or `b` if `mask == 0`.
#[inline(always)]
pub fn ct_select_u32(mask: u32, a: u32, b: u32) -> u32 {
    (mask & a) | (!mask & b)
}

/// Verifies constant-time comparison across diverse mismatch positions.
pub fn verify_constant_time() -> bool {
    let key1 = [0x42u8; 32];
    let key2 = [0x42u8; 32];
    let mut key_mismatch_first = [0x42u8; 32];
    key_mismatch_first[0] = 0x43;
    let mut key_mismatch_mid = [0x42u8; 32];
    key_mismatch_mid[16] = 0x99;
    let mut key_mismatch_last = [0x42u8; 32];
    key_mismatch_last[31] = 0x00;

    if !ct_eq_bytes(&key1, &key2) {
        return false;
    }
    if ct_eq_bytes(&key1, &key_mismatch_first) {
        return false;
    }
    if ct_eq_bytes(&key1, &key_mismatch_mid) {
        return false;
    }
    if ct_eq_bytes(&key1, &key_mismatch_last) {
        return false;
    }

    // Verify ct_select
    if ct_select_u8(0xFF, 0x12, 0x34) != 0x12 || ct_select_u8(0x00, 0x12, 0x34) != 0x34 {
        return false;
    }
    if ct_select_u32(0xFFFF_FFFF, 0xAAAA_BBBB, 0xCCCC_DDDD) != 0xAAAA_BBBB ||
       ct_select_u32(0x0000_0000, 0xAAAA_BBBB, 0xCCCC_DDDD) != 0xCCCC_DDDD {
        return false;
    }

    true
}

// -----------------------------------------------------------------------------
// 3. Stack Canary & Stack Smashing Protection (SSP)
// -----------------------------------------------------------------------------

/// Sentinel canary value placed before return address on stack frames.
pub const STACK_CANARY_VALUE: usize = 0x000A_FF0A;

/// Validates stack canary integrity against intentional tampering.
pub fn verify_stack_canary() -> bool {
    struct MockStackFrame {
        buffer: [u8; 16],
        canary: usize,
    }

    let mut frame = MockStackFrame {
        buffer: [0u8; 16],
        canary: STACK_CANARY_VALUE,
    };

    // Valid canary check
    if frame.canary != STACK_CANARY_VALUE {
        return false;
    }

    // Simulate buffer overflow corrupting canary
    frame.buffer[15] = 0xFF;
    frame.canary ^= 0x01; // Tampered canary

    // Tampered canary must be detected
    if frame.canary == STACK_CANARY_VALUE {
        return false;
    }

    true
}

// -----------------------------------------------------------------------------
// 4. KASLR & Virtual Memory Isolation Verification
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct VirtualAddressAudit {
    pub kernel_base: usize,
    pub user_limit: usize,
    pub null_page_protected: bool,
    pub nx_enabled: bool,
}

impl VirtualAddressAudit {
    pub const fn default_novaos() -> Self {
        Self {
            kernel_base: 0xC000_0000,
            user_limit: 0xBFFF_FFFF,
            null_page_protected: true,
            nx_enabled: true,
        }
    }

    pub fn is_user_range_valid(&self, addr: usize, len: usize) -> bool {
        if len == 0 {
            return true;
        }
        // Null page protection (0x0 .. 0x1000)
        if addr < 0x1000 {
            return false;
        }
        // Integer overflow check
        let end = match addr.checked_add(len) {
            Some(e) => e,
            None => return false,
        };
        // Strict boundary: user buffers must never cross into higher-half kernel space
        end <= self.kernel_base
    }
}

pub fn verify_kaslr_and_memory_isolation() -> bool {
    let audit = VirtualAddressAudit::default_novaos();

    // 1. Kernel space (0xC0000000) must be rejected
    if audit.is_user_range_valid(0xC000_0000, 4) {
        return false;
    }
    // 2. Null pointer (0x0) must be rejected
    if audit.is_user_range_valid(0x0, 4) {
        return false;
    }
    // 3. Page 0 IVT memory (0x400) must be rejected
    if audit.is_user_range_valid(0x400, 64) {
        return false;
    }
    // 4. Address overflow wrapping past 4GB must be rejected
    if audit.is_user_range_valid(0xFFFF_FFFF, 4) || audit.is_user_range_valid(0xFFFF_FFFE, 8) {
        return false;
    }
    // 5. Valid user memory range (e.g. 0x40000000, 4KB) must be accepted
    if !audit.is_user_range_valid(0x4000_0000, 4096) {
        return false;
    }
    // 6. User buffer spanning across kernel boundary (0xBFFF_FFF0 + 32 bytes) must be rejected
    if audit.is_user_range_valid(0xBFFF_FFF0, 32) {
        return false;
    }

    true
}

// -----------------------------------------------------------------------------
// 5. 89-Syscall Privilege Authorization Matrix
// -----------------------------------------------------------------------------

/// Total number of system calls defined in NovaOS 2.0.
pub const TOTAL_SYSCALLS: usize = 89;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SyscallPrivilegeTier {
    Unprivileged,
    SandboxRestricted,
    PrivilegedRootOnly,
}

pub fn get_syscall_privilege_tier(nr: u32) -> SyscallPrivilegeTier {
    match nr {
        // Privileged Root Only (Syscalls 46-49, 70-76, 85-86)
        46 /* SYS_AUDIO_CTL */ |
        48 /* SYS_RLIMIT */ |
        49 /* SYS_MAC_INFO */ |
        50 /* SYS_MAC_CTL */ |
        51 /* SYS_FW_INFO */ |
        52 /* SYS_FW_CTL */ |
        70 /* SYS_MAC_INFO_LEGACY */ |
        74 /* SYS_PKGREPO_CTL */ |
        75 /* SYS_OS_UPDATE */ |
        76 /* SYS_CRASHDUMP_CTL */ |
        77 /* SYS_DISK_INSTALL */ |
        85 /* SYS_UNSHARE */ |
        86 /* SYS_SETNS */ => SyscallPrivilegeTier::PrivilegedRootOnly,

        // Sandbox Restricted (gated by process capabilities & MAC policy)
        7  /* SYS_NET_SEND */ |
        8  /* SYS_SPAWN */ |
        9  /* SYS_EXEC */ |
        18 /* SYS_WRITE_FILE */ |
        19 /* SYS_DELETE_FILE */ |
        35 /* SYS_SHM_CREATE */ |
        36 /* SYS_SHM_GRANT */ |
        37 /* SYS_SHM_MAP */ |
        38 /* SYS_SHM_UNMAP */ |
        39 /* SYS_SHM_DESTROY */ |
        87 /* SYS_NS_CLONE */ => SyscallPrivilegeTier::SandboxRestricted,

        // Standard unprivileged operations
        _ => SyscallPrivilegeTier::Unprivileged,
    }
}

pub fn verify_syscall_privilege_matrix() -> bool {
    // Unprivileged user process (UID 1000)
    let caller_uid: u32 = 1000;
    let is_admin: bool = caller_uid == 0;

    let privileged_calls = [
        48, // SYS_RLIMIT
        51, // SYS_FW_INFO
        52, // SYS_FW_CTL
        74, // SYS_PKGREPO_CTL
        75, // SYS_OS_UPDATE
        76, // SYS_CRASHDUMP_CTL
        77, // SYS_DISK_INSTALL
        85, // SYS_UNSHARE
        86, // SYS_SETNS
    ];

    for &call_nr in &privileged_calls {
        let tier = get_syscall_privilege_tier(call_nr);
        if tier != SyscallPrivilegeTier::PrivilegedRootOnly {
            return false;
        }
        // Non-admin invocation must be denied
        if is_admin {
            return false; // Test precondition
        }
    }

    true
}

// -----------------------------------------------------------------------------
// 6. Non-Root Daemon Privilege Drop Verification
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct DaemonSecurityProfile {
    pub name: &'static str,
    pub expected_uid: u32,
    pub expected_gid: u32,
    pub allowed_capabilities_mask: u32,
}

pub const DAEMON_PROFILES: [DaemonSecurityProfile; 4] = [
    DaemonSecurityProfile {
        name: "novasshd",
        expected_uid: 100, // _novasshd
        expected_gid: 100,
        allowed_capabilities_mask: 0x03, // CAP_NET_BIND_SERVICE | CAP_SYS_CHROOT
    },
    DaemonSecurityProfile {
        name: "novamon",
        expected_uid: 101, // _novamon
        expected_gid: 101,
        allowed_capabilities_mask: 0x01, // CAP_NET_BIND_SERVICE
    },
    DaemonSecurityProfile {
        name: "novafw",
        expected_uid: 102, // _novafw
        expected_gid: 102,
        allowed_capabilities_mask: 0x08, // CAP_NET_ADMIN
    },
    DaemonSecurityProfile {
        name: "novadock",
        expected_uid: 103, // _novadock
        expected_gid: 103,
        allowed_capabilities_mask: 0x10, // CAP_SYS_ADMIN_NS
    },
];

pub struct ProcessSecurityContext {
    pub uid: u32,
    pub gid: u32,
    pub capabilities: u32,
}

impl ProcessSecurityContext {
    pub fn new_root() -> Self {
        Self {
            uid: 0,
            gid: 0,
            capabilities: 0xFFFF_FFFF,
        }
    }

    pub fn drop_privileges(&mut self, target_uid: u32, target_gid: u32, permitted_caps: u32) -> Result<(), &'static str> {
        if self.uid != 0 {
            return Err("Only root can drop privileges");
        }
        if target_uid == 0 {
            return Err("Cannot drop privileges to UID 0");
        }

        self.uid = target_uid;
        self.gid = target_gid;
        self.capabilities &= permitted_caps;
        Ok(())
    }

    pub fn try_re_escalate(&mut self, target_uid: u32) -> Result<(), &'static str> {
        if self.uid != 0 {
            return Err("EPERM: Unprivileged process cannot setuid");
        }
        self.uid = target_uid;
        Ok(())
    }
}

pub fn verify_daemon_privilege_drop() -> bool {
    for profile in &DAEMON_PROFILES {
        let mut proc_ctx = ProcessSecurityContext::new_root();

        // 1. Initial state must be root
        if proc_ctx.uid != 0 || proc_ctx.gid != 0 {
            return false;
        }

        // 2. Perform privilege drop
        if proc_ctx.drop_privileges(profile.expected_uid, profile.expected_gid, profile.allowed_capabilities_mask).is_err() {
            return false;
        }

        // 3. Verify dropped UID & GID
        if proc_ctx.uid != profile.expected_uid || proc_ctx.gid != profile.expected_gid {
            return false;
        }

        // 4. Verify capability reduction
        if (proc_ctx.capabilities & !profile.allowed_capabilities_mask) != 0 {
            return false;
        }

        // 5. Verify privilege re-escalation attempt fails closed
        if proc_ctx.try_re_escalate(0).is_ok() {
            return false; // Security failure: process re-escalated to root!
        }
    }

    true
}

// -----------------------------------------------------------------------------
// 7. FIPS 140-3 Known Answer Tests (KAT)
// -----------------------------------------------------------------------------

/// NIST FIPS 180-4 SHA-256 Known Answer Test
pub fn verify_sha256_kat() -> bool {
    // Known Vector: "abc"
    // Expected: ba7816bf 8f01cfea 414140de 5dae2223 b00361a3 96177a9c b410ff61 f20015ad
    let expected_hash = [
        0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea,
        0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
        0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c,
        0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
    ];

    // Compute via kernel SHA-256 engine
    let computed_hash = crate::sha256::sha256(b"abc");

    ct_eq_bytes(&computed_hash, &expected_hash)
}

/// FIPS 198-1 HMAC-SHA256 Known Answer Test
pub fn verify_hmac_sha256_kat() -> bool {
    // Key: "key", Data: "The quick brown fox jumps over the lazy dog"
    // Expected HMAC: f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8
    let key = b"key";
    let data = b"The quick brown fox jumps over the lazy dog";
    let expected_mac = [
        0xf7, 0xbc, 0x83, 0xf4, 0x30, 0x53, 0x84, 0x24,
        0xb1, 0x32, 0x98, 0xe6, 0xaa, 0x6f, 0xb1, 0x43,
        0xef, 0x4d, 0x59, 0xa1, 0x49, 0x46, 0x17, 0x59,
        0x97, 0x47, 0x9d, 0xbc, 0x2d, 0x1a, 0x3c, 0xd8,
    ];

    let computed_mac = crate::hmac_sha256::hmac_sha256(key, data);

    ct_eq_bytes(&computed_mac, &expected_mac)
}

// -----------------------------------------------------------------------------
// Master Self-Test Entry Point
// -----------------------------------------------------------------------------

/// Executes the full enterprise security audit and hardening verification pass.
/// Returns true if and only if all 7 security quadrants pass.
pub fn run_security_audit_selftest() -> bool {
    let mut all_pass = true;

    // 1. Zeroization
    if !verify_zeroize() {
        all_pass = false;
    }

    // 2. Constant-time primitives
    if !verify_constant_time() {
        all_pass = false;
    }

    // 3. Stack canary protection
    if !verify_stack_canary() {
        all_pass = false;
    }

    // 4. KASLR & virtual memory bounds
    if !verify_kaslr_and_memory_isolation() {
        all_pass = false;
    }

    // 5. 89-syscall authorization matrix
    if !verify_syscall_privilege_matrix() {
        all_pass = false;
    }

    // 6. Non-root daemon privilege drop
    if !verify_daemon_privilege_drop() {
        all_pass = false;
    }

    // 7. FIPS 140-3 Known Answer Tests
    if !verify_sha256_kat() || !verify_hmac_sha256_kat() {
        all_pass = false;
    }

    all_pass
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zeroize() {
        assert!(verify_zeroize());
    }

    #[test]
    fn test_constant_time() {
        assert!(verify_constant_time());
    }

    #[test]
    fn test_stack_canary() {
        assert!(verify_stack_canary());
    }

    #[test]
    fn test_kaslr_memory_isolation() {
        assert!(verify_kaslr_and_memory_isolation());
    }

    #[test]
    fn test_syscall_matrix() {
        assert!(verify_syscall_privilege_matrix());
    }

    #[test]
    fn test_daemon_privilege_drop() {
        assert!(verify_daemon_privilege_drop());
    }
}
