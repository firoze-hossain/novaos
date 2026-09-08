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
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe {
        serial_puts(b"[RUST PANIC] kernel-side Rust code panicked\n\0".as_ptr());
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
