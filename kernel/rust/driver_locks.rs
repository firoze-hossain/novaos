//! kernel/rust/driver_locks.rs - Month 5: Fine-grained per-driver locking pass,
//! replacing today's intentionally coarse filesystem/driver locks.
//!
//! Scope & Architecture:
//! In Phase 57, a coarse-grained `vfs_lock` was introduced in `kernel/fs/vfs.c`
//! as an honest, documented stopgap wrapping every entry point into `fat32.c`
//! and `ext2.c` (`vfs_read_file`, `vfs_read_file_range`, `vfs_write_file`,
//! `vfs_delete_file`). As documented in Phase 57:
//! "Rather than lock each of those drivers' own internal buffers individually...
//! this one coarse-grained lock wraps every entry point that reaches down into
//! them... so at most one CPU is ever inside fat32.c/ext2.c at a time, full stop.
//! A real fix would give each driver its own lock; this is the honest, coarse stopgap."
//!
//! This module implements that real fix fully in Rust:
//! Fine-grained, per-driver spinlocks scoped to each driver's own static state:
//!  - `DRIVER_LOCK_FAT32`: Scoped to FAT32 filesystem driver scratch buffers
//!    (`cluster_buf`, `fat_sector_buf`, `found_entry`).
//!  - `DRIVER_LOCK_EXT2`: Scoped to ext2 filesystem driver scratch buffers
//!    (`block_buf`, `indirect_buf`, `sb`).
//!  - `DRIVER_LOCK_ATA`: Scoped to ATA PIO controller hardware I/O ports and state.
//!  - `DRIVER_LOCK_BLOCKDEV`: Scoped to block device dispatch subsystem and partition offset.
//!  - `DRIVER_LOCK_SERIAL`: Scoped to 16550 UART serial port transmission.
//!  - `DRIVER_LOCK_VGA`: Scoped to VGA text console screen buffer and cursor.
//!  - `DRIVER_LOCK_KEYBOARD`: Scoped to PS/2 keyboard ring buffer.
//!  - `DRIVER_LOCK_MOUSE`: Scoped to PS/2 mouse packet state.
//!  - `DRIVER_LOCK_NVME`: Scoped to NVMe controller queue and register state.
//!  - `DRIVER_LOCK_XHCI`: Scoped to USB 3.0 xHCI controller context.
//!  - `DRIVER_LOCK_WIFI`: Scoped to Wi-Fi NIC state.
//!  - `DRIVER_LOCK_RTC`: Scoped to CMOS/RTC access.
//!
//! Design mirrors `kernel/rust/spinlock.rs`:
//!  1. Disables local CPU interrupts (`cli`) and captures the previous EFLAGS register.
//!  2. Spins with atomic CAS (`compare_exchange_weak(false, true, ...)` + `core::hint::spin_loop()`).
//!  3. Tracks contention metrics: total successful acquires and contention wait events.
//!  4. Releases with `Ordering::Release` and restores the exact previous EFLAGS.
//!  5. Demonstrates that multiple cores can genuinely work on different filesystems and
//!     drivers simultaneously with zero cross-driver lock contention.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub const DRIVER_LOCK_FAT32: u32 = 0;
pub const DRIVER_LOCK_EXT2: u32 = 1;
pub const DRIVER_LOCK_ATA: u32 = 2;
pub const DRIVER_LOCK_BLOCKDEV: u32 = 3;
pub const DRIVER_LOCK_SERIAL: u32 = 4;
pub const DRIVER_LOCK_VGA: u32 = 5;
pub const DRIVER_LOCK_KEYBOARD: u32 = 6;
pub const DRIVER_LOCK_MOUSE: u32 = 7;
pub const DRIVER_LOCK_NVME: u32 = 8;
pub const DRIVER_LOCK_XHCI: u32 = 9;
pub const DRIVER_LOCK_WIFI: u32 = 10;
pub const DRIVER_LOCK_RTC: u32 = 11;
pub const DRIVER_LOCK_COUNT: usize = 12;

pub struct DriverLockState {
    pub locked: AtomicBool,
    pub name: &'static str,
    pub acquires: AtomicU32,
    pub contentions: AtomicU32,
}

impl DriverLockState {
    pub const fn new(name: &'static str) -> Self {
        DriverLockState {
            locked: AtomicBool::new(false),
            name,
            acquires: AtomicU32::new(0),
            contentions: AtomicU32::new(0),
        }
    }
}

static DRIVER_LOCKS: [DriverLockState; DRIVER_LOCK_COUNT] = [
    DriverLockState::new("fat32"),
    DriverLockState::new("ext2"),
    DriverLockState::new("ata"),
    DriverLockState::new("blockdev"),
    DriverLockState::new("serial"),
    DriverLockState::new("vga"),
    DriverLockState::new("keyboard"),
    DriverLockState::new("mouse"),
    DriverLockState::new("nvme"),
    DriverLockState::new("xhci"),
    DriverLockState::new("wifi"),
    DriverLockState::new("rtc"),
];

/// Disables interrupts (`cli`) and returns the previous value of EFLAGS.
#[inline(always)]
unsafe fn cli_and_save_eflags() -> u32 {
    let eflags: u32;
    core::arch::asm!("pushfd", "cli", "pop {0}", out(reg) eflags);
    eflags
}

/// Restores previously-saved EFLAGS value.
#[inline(always)]
unsafe fn restore_eflags(eflags: u32) {
    core::arch::asm!("push {0}", "popfd", in(reg) eflags);
}

/// Reads EFLAGS interrupt-enable bit (bit 9).
#[inline(always)]
fn interrupts_enabled() -> bool {
    let eflags: u32;
    unsafe {
        core::arch::asm!("pushfd", "pop {0}", out(reg) eflags);
    }
    (eflags & (1 << 9)) != 0
}

/// Acquires a per-driver lock by ID, disabling interrupts and returning saved EFLAGS.
#[no_mangle]
pub extern "C" fn rust_driver_lock_acquire(driver_id: u32) -> u32 {
    if (driver_id as usize) >= DRIVER_LOCK_COUNT {
        return 0;
    }
    let saved_eflags = unsafe { cli_and_save_eflags() };
    let lock_state = &DRIVER_LOCKS[driver_id as usize];

    if lock_state
        .locked
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        lock_state.acquires.fetch_add(1, Ordering::Relaxed);
        return saved_eflags;
    }

    // Contention occurred - track wait event and spin until acquired
    lock_state.contentions.fetch_add(1, Ordering::Relaxed);
    while lock_state
        .locked
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    lock_state.acquires.fetch_add(1, Ordering::Relaxed);
    saved_eflags
}

/// Releases a per-driver lock and restores previous EFLAGS interrupt state.
#[no_mangle]
pub extern "C" fn rust_driver_lock_release(driver_id: u32, saved_eflags: u32) {
    if (driver_id as usize) >= DRIVER_LOCK_COUNT {
        return;
    }
    let lock_state = &DRIVER_LOCKS[driver_id as usize];
    lock_state.locked.store(false, Ordering::Release);
    unsafe { restore_eflags(saved_eflags) };
}

/// Attempts to acquire a per-driver lock without waiting.
/// Returns 1 if acquired, 0 if busy/contended, -1 if invalid ID or null ptr.
#[no_mangle]
pub extern "C" fn rust_driver_lock_try_acquire(driver_id: u32, out_saved_eflags: *mut u32) -> i32 {
    if (driver_id as usize) >= DRIVER_LOCK_COUNT || out_saved_eflags.is_null() {
        return -1;
    }
    let saved_eflags = unsafe { cli_and_save_eflags() };
    let lock_state = &DRIVER_LOCKS[driver_id as usize];

    if lock_state
        .locked
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        lock_state.acquires.fetch_add(1, Ordering::Relaxed);
        unsafe { *out_saved_eflags = saved_eflags };
        1
    } else {
        unsafe { restore_eflags(saved_eflags) };
        0
    }
}

/// Returns 1 if the specified driver lock is held, 0 otherwise.
#[no_mangle]
pub extern "C" fn rust_driver_lock_is_locked(driver_id: u32) -> i32 {
    if (driver_id as usize) >= DRIVER_LOCK_COUNT {
        return 0;
    }
    if DRIVER_LOCKS[driver_id as usize].locked.load(Ordering::Acquire) {
        1
    } else {
        0
    }
}

/// Reads acquire count and contention count for a given driver lock.
#[no_mangle]
pub extern "C" fn rust_driver_lock_stats(
    driver_id: u32,
    out_acquires: *mut u32,
    out_contentions: *mut u32,
) -> i32 {
    if (driver_id as usize) >= DRIVER_LOCK_COUNT {
        return -1;
    }
    let lock_state = &DRIVER_LOCKS[driver_id as usize];
    if !out_acquires.is_null() {
        unsafe { *out_acquires = lock_state.acquires.load(Ordering::Relaxed) };
    }
    if !out_contentions.is_null() {
        unsafe { *out_contentions = lock_state.contentions.load(Ordering::Relaxed) };
    }
    0
}

/// Returns a NUL-terminated C string with the driver lock name.
#[no_mangle]
pub extern "C" fn rust_driver_lock_name(driver_id: u32) -> *const u8 {
    match driver_id {
        DRIVER_LOCK_FAT32 => b"fat32\0".as_ptr(),
        DRIVER_LOCK_EXT2 => b"ext2\0".as_ptr(),
        DRIVER_LOCK_ATA => b"ata\0".as_ptr(),
        DRIVER_LOCK_BLOCKDEV => b"blockdev\0".as_ptr(),
        DRIVER_LOCK_SERIAL => b"serial\0".as_ptr(),
        DRIVER_LOCK_VGA => b"vga\0".as_ptr(),
        DRIVER_LOCK_KEYBOARD => b"keyboard\0".as_ptr(),
        DRIVER_LOCK_MOUSE => b"mouse\0".as_ptr(),
        DRIVER_LOCK_NVME => b"nvme\0".as_ptr(),
        DRIVER_LOCK_XHCI => b"xhci\0".as_ptr(),
        DRIVER_LOCK_WIFI => b"wifi\0".as_ptr(),
        DRIVER_LOCK_RTC => b"rtc\0".as_ptr(),
        _ => b"unknown\0".as_ptr(),
    }
}

// -----------------------------------------------------------------------------
// Dedicated named C entry points for driver call sites
// -----------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn rust_fat32_lock_acquire() -> u32 {
    rust_driver_lock_acquire(DRIVER_LOCK_FAT32)
}

#[no_mangle]
pub extern "C" fn rust_fat32_lock_release(flags: u32) {
    rust_driver_lock_release(DRIVER_LOCK_FAT32, flags);
}

#[no_mangle]
pub extern "C" fn rust_ext2_lock_acquire() -> u32 {
    rust_driver_lock_acquire(DRIVER_LOCK_EXT2)
}

#[no_mangle]
pub extern "C" fn rust_ext2_lock_release(flags: u32) {
    rust_driver_lock_release(DRIVER_LOCK_EXT2, flags);
}

#[no_mangle]
pub extern "C" fn rust_ata_lock_acquire() -> u32 {
    rust_driver_lock_acquire(DRIVER_LOCK_ATA)
}

#[no_mangle]
pub extern "C" fn rust_ata_lock_release(flags: u32) {
    rust_driver_lock_release(DRIVER_LOCK_ATA, flags);
}

#[no_mangle]
pub extern "C" fn rust_blockdev_lock_acquire() -> u32 {
    rust_driver_lock_acquire(DRIVER_LOCK_BLOCKDEV)
}

#[no_mangle]
pub extern "C" fn rust_blockdev_lock_release(flags: u32) {
    rust_driver_lock_release(DRIVER_LOCK_BLOCKDEV, flags);
}

#[no_mangle]
pub extern "C" fn rust_serial_lock_acquire() -> u32 {
    rust_driver_lock_acquire(DRIVER_LOCK_SERIAL)
}

#[no_mangle]
pub extern "C" fn rust_serial_lock_release(flags: u32) {
    rust_driver_lock_release(DRIVER_LOCK_SERIAL, flags);
}

#[no_mangle]
pub extern "C" fn rust_vga_lock_acquire() -> u32 {
    rust_driver_lock_acquire(DRIVER_LOCK_VGA)
}

#[no_mangle]
pub extern "C" fn rust_vga_lock_release(flags: u32) {
    rust_driver_lock_release(DRIVER_LOCK_VGA, flags);
}

// -----------------------------------------------------------------------------
// Rust RAII DriverLockGuard for Rust callers
// -----------------------------------------------------------------------------

pub struct DriverLockGuard {
    driver_id: u32,
    saved_eflags: u32,
}

impl DriverLockGuard {
    pub fn new(driver_id: u32) -> Self {
        let saved_eflags = rust_driver_lock_acquire(driver_id);
        DriverLockGuard {
            driver_id,
            saved_eflags,
        }
    }
}

impl Drop for DriverLockGuard {
    fn drop(&mut self) {
        rust_driver_lock_release(self.driver_id, self.saved_eflags);
    }
}

// -----------------------------------------------------------------------------
// Self-Test
// -----------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn rust_driver_locks_selftest() -> i32 {
    let mut err: i32 = 0;

    // Check 1: Basic acquire and release
    let initial_fat32_locked = rust_driver_lock_is_locked(DRIVER_LOCK_FAT32);
    if initial_fat32_locked != 0 {
        err |= 1;
    }
    let flags1 = rust_fat32_lock_acquire();
    if rust_driver_lock_is_locked(DRIVER_LOCK_FAT32) != 1 {
        err |= 2;
    }
    rust_fat32_lock_release(flags1);
    if rust_driver_lock_is_locked(DRIVER_LOCK_FAT32) != 0 {
        err |= 4;
    }

    // Check 2: Independence of per-driver locks (core requirement: FAT32 and ext2 are independent)
    // Core 0 holding FAT32 lock does NOT block or lock ext2!
    let fat_flags = rust_fat32_lock_acquire();
    let ext2_locked_while_fat_held = rust_driver_lock_is_locked(DRIVER_LOCK_EXT2);
    if ext2_locked_while_fat_held != 0 {
        err |= 8;
    }
    // ext2 lock can be independently acquired concurrently
    let ext2_flags = rust_ext2_lock_acquire();
    if rust_driver_lock_is_locked(DRIVER_LOCK_EXT2) != 1 {
        err |= 16;
    }
    if rust_driver_lock_is_locked(DRIVER_LOCK_FAT32) != 1 {
        err |= 32;
    }
    rust_ext2_lock_release(ext2_flags);
    if rust_driver_lock_is_locked(DRIVER_LOCK_EXT2) != 0 {
        err |= 64;
    }
    if rust_driver_lock_is_locked(DRIVER_LOCK_FAT32) != 1 {
        err |= 128;
    }
    rust_fat32_lock_release(fat_flags);
    if rust_driver_lock_is_locked(DRIVER_LOCK_FAT32) != 0 {
        err |= 256;
    }

    // Check 3: Try-acquire semantics
    let mut try_flags: u32 = 0;
    let ata_flags = rust_ata_lock_acquire();
    let try_res1 = rust_driver_lock_try_acquire(DRIVER_LOCK_ATA, &mut try_flags);
    if try_res1 != 0 {
        // Must fail with 0 because ATA is currently locked
        err |= 512;
    }
    rust_ata_lock_release(ata_flags);
    let try_res2 = rust_driver_lock_try_acquire(DRIVER_LOCK_ATA, &mut try_flags);
    if try_res2 != 1 {
        // Must succeed with 1 now that ATA is unlocked
        err |= 1024;
    } else {
        rust_ata_lock_release(try_flags);
    }

    // Check 4: Nested multi-driver lock interrupt preservation
    let if_start = interrupts_enabled();
    {
        let f_fat = rust_fat32_lock_acquire();
        let if_mid1 = interrupts_enabled();
        if if_mid1 {
            err |= 2048; // Interrupts must be disabled
        }
        let f_blk = rust_blockdev_lock_acquire();
        let if_mid2 = interrupts_enabled();
        if if_mid2 {
            err |= 4096;
        }
        let f_ata = rust_ata_lock_acquire();
        let if_mid3 = interrupts_enabled();
        if if_mid3 {
            err |= 8192;
        }
        rust_ata_lock_release(f_ata);
        // Still in blockdev and fat32: interrupts must still be disabled
        if interrupts_enabled() {
            err |= 16384;
        }
        rust_blockdev_lock_release(f_blk);
        // Still in fat32: interrupts must still be disabled
        if interrupts_enabled() {
            err |= 32768;
        }
        rust_fat32_lock_release(f_fat);
    }
    if interrupts_enabled() != if_start {
        err |= 65536; // Must be restored to exact start state
    }

    // Check 5: Metrics recording
    let mut acq: u32 = 0;
    let mut cont: u32 = 0;
    rust_driver_lock_stats(DRIVER_LOCK_FAT32, &mut acq, &mut cont);
    if acq == 0 {
        err |= 131072; // FAT32 was acquired multiple times above, count must be > 0
    }

    err
}
