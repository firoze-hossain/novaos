//! kernel/rust/lib.rs - NovaOS's first kernel-side Rust code
//! (Phase 35).
//!
//! Unlike userland Rust (userland/ping-rs, userland/store-rs), this
//! is `#![no_main]`-less: there's no "process" or crt0 entry point
//! here - the existing C kernel calls into exported functions
//! (`extern "C"`), the opposite direction from userland, where C's
//! crt0.asm calls into Rust's `main()`. Built with the same
//! tools/rust-sysroot/ toolchain Phase 33 established, no changes to
//! that infrastructure needed - the risk here is entirely in linking
//! this into the kernel's own binary (a single ELF with a Multiboot
//! header that must sit within its first 8KB, per tools/linker.ld)
//! rather than a standalone userland ELF, and in what a bug here can
//! reach: kernel-side Rust runs in ring 0, with no memory protection
//! from the rest of the kernel, unlike a ring-3 Rust program's bugs
//! being contained to that one process.
//!
//! Scope for this phase: kept deliberately small and low-risk - a
//! provable, working integration (a real computed value round-
//! tripped through Rust and logged), not a rewrite of any existing,
//! working C subsystem (per this project's standing rule: what's
//! already done in C stays in C; new work is attempted in Rust
//! first).

#![no_std]

mod pipe;
mod spinlock;
mod virtio_blk;
mod net_irq;
mod acpi;
mod virtio_net;
mod users;
mod sha256;
mod hmac_sha256;
mod pbkdf2;
mod journal;
mod crashdump;
mod apic;
mod tcp;
mod udp;
mod virtiogpu;
mod virgl;
mod shm;
mod msg;
mod mac;
mod firewall;
mod mixer;
mod http;
mod pkgsign_core;
mod pkgsign;
mod dynlink;
mod growtable;
mod klog;
mod pkgrepo;
mod driver_locks;
mod atomic_update;
mod disk_installer;
mod services;
mod devman;
mod procman;

#[path = "../src/arch/x86_64/paging.rs"]
pub mod paging64;

#[path = "../src/arch/x86_64/syscall.rs"]
pub mod syscall64;

#[path = "../../boot/uefi-loader/src/lib.rs"]
pub mod uefi_loader;

#[path = "../src/mm/vmm.rs"]
pub mod vmm;

#[path = "../src/mm/fault.rs"]
pub mod fault;

#[path = "../src/sched/scheduler.rs"]
pub mod scheduler64;

#[path = "../src/drivers/storage/nvme.rs"]
pub mod nvme2;

#[path = "../src/drivers/usb/mod.rs"]
pub mod usb;

#[path = "../src/fs/vfs/mod.rs"]
pub mod vfs;

#[path = "../src/fs/novafs/mod.rs"]
pub mod novafs;

#[path = "../src/io/uring.rs"]
pub mod uring;

#[path = "../src/mm/pagecache.rs"]
pub mod pagecache;

#[path = "../src/mm/swap.rs"]
pub mod swap;

#[path = "../src/compat/posix.rs"]
pub mod posix_compat;

#[path = "../src/drivers/gpu/mod.rs"]
pub mod gpu;

#[path = "../src/drivers/audio/mod.rs"]
pub mod audio;

#[path = "../src/syscall/mod.rs"]
pub mod syscall;

use core::panic::PanicInfo;

extern "C" {
    fn serial_puts(s: *const u8);
}

/// A kernel-side Rust panic is far more serious than a userland one -
/// it runs in ring 0 with no isolation from the rest of the kernel,
/// so there's no "just this process hangs" containment. Logs to the
/// serial port (the same one kernel_log()/serial_printf() already use
/// for every other boot marker this project's own test harness greps
/// for) before halting, so a kernel-side Rust panic is at least
/// observable rather than a silent, undiagnosable hang - matching
/// this project's existing kernel panic handler's own intent
/// (kernel/init/main.c's panic path), just for the Rust side
/// specifically.
///
/// Also reports the panic's own file:line (`core::panic::Location`,
/// always available in a debug or release build - unlike the panic
/// *message* itself, which needs `core::fmt` formatting machinery
/// this freestanding, allocator-optional environment doesn't carry).
/// A permanent addition, not a one-off debugging aid: found directly
/// useful investigating a real, non-deterministic memory-corruption
/// bug in this kernel's own SYS_EXEC/ELF-loading path (see
/// PROGRESS.md's own notes on that investigation) - a bare "kernel-
/// side Rust code panicked" with no location was the first thing that
/// made narrowing that bug down slower than it needed to be. Written
/// with a fixed, bounded stack buffer and manual byte-by-byte
/// formatting rather than `core::fmt::Write` - deliberately: this
/// runs in a context where something has already gone wrong badly
/// enough to panic, so pulling in more of Rust's own formatting
/// machinery here is the wrong tradeoff versus a small, easy-to-
/// audit hand-written loop.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe {
        serial_puts(b"[RUST PANIC] kernel-side Rust code panicked\n\0".as_ptr());
        if let Some(loc) = info.location() {
            // 200 bytes is generous for "[RUST PANIC] at " (16) +
            // any real source path in this tree (the longest,
            // kernel/rust/net_irq.rs, is 24) + ":" + up to 10 digits
            // of line number + "\n" + the NUL terminator - real
            // headroom, not a tight fit, so the saturating writes
            // below (each stops rather than overflows once `pos`
            // nears the end) are a defensive backstop, not something
            // any real call here is expected to actually hit.
            let mut buf = [0u8; 200];
            let mut pos = 0usize;

            let mut write_bytes = |bytes: &[u8]| {
                for &b in bytes {
                    if pos < buf.len() - 1 {
                        buf[pos] = b;
                        pos += 1;
                    }
                }
            };

            write_bytes(b"[RUST PANIC] at ");
            write_bytes(loc.file().as_bytes());
            write_bytes(b":");

            let mut line = loc.line();
            let mut digits = [0u8; 10];
            let mut ndig = 0usize;
            if line == 0 {
                digits[0] = b'0';
                ndig = 1;
            } else {
                while line > 0 && ndig < digits.len() {
                    digits[ndig] = b'0' + (line % 10) as u8;
                    line /= 10;
                    ndig += 1;
                }
            }
            for i in (0..ndig).rev() {
                write_bytes(&[digits[i]]);
            }
            write_bytes(b"\n");

            buf[pos] = 0;
            serial_puts(buf.as_ptr());
        }
    }
    loop {
        unsafe {
            core::arch::asm!("cli", "hlt");
        }
    }
}

/// Phase 35's proof of concept: a real, observable computation
/// (not just "compiles"), called from kernel_main() and logged via
/// the existing serial_puts() - confirms Rust object code correctly
/// links into the kernel's own ELF (built by tools/linker.ld, with
/// its Multiboot-header-placement constraint) and that a value
/// computed in Rust is correctly visible to C on the other side of
/// the FFI boundary.
#[no_mangle]
pub extern "C" fn rust_kernel_selftest_add(a: i32, b: i32) -> i32 {
    let mut sum: i32 = 0;
    let mut i = 0;
    while i < 2 {
        sum += if i == 0 { a } else { b };
        i += 1;
    }
    sum
}

/// NovaOS 2.0 (Month 1 - Foundation): Demand Paging & Copy-on-Write (CoW) Virtual Memory Engine
/// self-test callable from kernel boot sequence.
#[no_mangle]
pub extern "C" fn rust_demand_paging_cow_selftest() -> i32 {
    fault::run_demand_paging_cow_selftest()
}

/// NovaOS 2.0 (Month 1 - Foundation): Preemptive Tickless SMP Scheduler with Priority Classes
/// self-test callable from kernel boot sequence.
#[no_mangle]
pub extern "C" fn rust_smp_scheduler_selftest() -> i32 {
    scheduler64::run_smp_scheduler_selftest()
}

/// NovaOS 2.0 (Month 1 - Foundation): Pure Rust NVMe 2.0 PCIe Storage Controller Driver
/// self-test callable from kernel boot sequence.
#[no_mangle]
pub extern "C" fn rust_nvme2_selftest() -> i32 {
    nvme2::run_nvme2_selftest()
}

/// NovaOS 2.0 (Month 1 - Foundation): Pure Rust xHCI USB 3.2 Host Controller & HID Stack
/// self-test callable from kernel boot sequence.
#[no_mangle]
pub extern "C" fn rust_xhci_hid_selftest() -> i32 {
    usb::run_xhci_hid_selftest()
}

/// NovaOS 2.0 (Month 2 - Storage & Async I/O): Concurrent VFS with Lockless Dentry Cache & Async Inotify
/// self-test callable from kernel boot sequence.
#[no_mangle]
pub extern "C" fn rust_vfs_selftest() -> i32 {
    vfs::run_vfs_selftest()
}

/// NovaOS 2.0 (Month 2 - Storage & Async I/O): Pure Rust Enterprise Filesystem (NovaFS)
/// with B-Tree CoW, 64-bit extents, SHA-256 integrity, atomic snapshots & subvolumes.
#[no_mangle]
pub extern "C" fn rust_novafs_selftest() -> i32 {
    novafs::run_novafs_selftest()
}

/// NovaOS 2.0 (Month 2 - Storage & Async I/O): Asynchronous I/O Ring Engine (nova_uring)
/// self-test callable from kernel boot sequence.
#[no_mangle]
pub extern "C" fn rust_uring_selftest() -> i32 {
    uring::run_uring_selftest()
}

/// NovaOS 2.0 (Month 2 - Storage & Async I/O): Kernel Memory Page Cache & Anonymous Swap Subsystem
/// self-test callable from kernel boot sequence.
#[no_mangle]
pub extern "C" fn rust_pagecache_swap_selftest() -> i32 {
    let res_pc = pagecache::run_pagecache_selftest();
    if res_pc != 0 {
        return res_pc;
    }
    let res_sw = swap::run_swap_selftest();
    if res_sw != 0 {
        return 100 + res_sw;
    }
    0
}

/// NovaOS 2.0 (Month 2 - Storage & Async I/O): Linux/POSIX Compatibility Layer (novalibc & POSIX ABI)
/// self-test callable from kernel boot sequence.
#[no_mangle]
pub extern "C" fn rust_posix_compat_selftest() -> i32 {
    posix_compat::run_posix_compat_selftest()
}

/// NovaOS 2.0 (Month 4 - Graphics & DRM): DRM/KMS Kernel Mode-Setting & Display Plane Pipeline
/// self-test callable from kernel boot sequence.
#[no_mangle]
pub extern "C" fn rust_drm_kms_selftest() -> i32 {
    gpu::run_drm_kms_selftest()
}

/// NovaOS 2.0 (Month 4 - Graphics & DRM): VirtIO-GPU 3D Hardware Acceleration & Virgl Mesa Bridge
/// self-test callable from kernel boot sequence.
#[no_mangle]
pub extern "C" fn rust_virtio_3d_selftest() -> i32 {
    gpu::run_virtio_3d_selftest()
}

/// NovaOS 2.0 (Month 4 - Graphics & DRM): Zero-Copy DMA-BUF Framebuffer Sharing & GPU Memory Allocator
/// self-test callable from kernel boot sequence.
#[no_mangle]
pub extern "C" fn rust_dmabuf_selftest() -> i32 {
    syscall::run_dmabuf_selftest()
}

/// NovaOS 2.0 (Month 4 - Graphics & DRM): C-ABI DMA-BUF Syscall Dispatch Hook
#[no_mangle]
pub extern "C" fn rust_sys_dmabuf_dispatch(
    caller_pid: u32,
    syscall_nr: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
) -> isize {
    syscall::dispatch_drm_syscall(caller_pid, syscall_nr, arg1, arg2, arg3)
}

/// NovaOS 2.0 (Month 5 - Desktop Experience): Intel High Definition Audio (HDA / Azalia) Subsystem
/// self-test callable from kernel boot sequence.
#[no_mangle]
pub extern "C" fn rust_hda_selftest() -> i32 {
    audio::run_hda_selftest()
}

/// NovaOS 2.0 (Month 5 - Desktop Experience): C-ABI Audio PCM Playback write handler
#[no_mangle]
pub extern "C" fn rust_hda_write_pcm(buf: *const u8, len: usize) -> isize {
    if buf.is_null() || len == 0 {
        return 0;
    }
    let slice = unsafe { core::slice::from_raw_parts(buf, len) };
    audio::pcm_playback_write(slice) as isize
}

/// NovaOS 2.0 (Month 5 - Desktop Experience): C-ABI Audio PCM Capture read handler
#[no_mangle]
pub extern "C" fn rust_hda_read_pcm(buf: *mut u8, len: usize) -> isize {
    if buf.is_null() || len == 0 {
        return 0;
    }
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, len) };
    audio::pcm_capture_read(slice) as isize
}

/// NovaOS 2.0 (Month 5 - Desktop Experience): C-ABI Audio PCM Ioctl handler
#[no_mangle]
pub extern "C" fn rust_hda_ioctl(cmd: u32, arg: u64) -> isize {
    audio::pcm_playback_ioctl(cmd, arg)
}
