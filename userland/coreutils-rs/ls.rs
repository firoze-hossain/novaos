//! ls.rs - a genuine, standalone ring-3 `ls` program, written in Rust
//! (Phase 59).
//!
//! Lists every file on the mounted filesystem via SYS_LIST_FILES,
//! which already returns "NAME SIZE\n" lines (see kernel/arch/x86/cpu/
//! syscall.c's handle_list_files()) - this program's own job is just
//! to ask for that listing and print it, the same shape as the
//! existing ring-3 shell's own inline `cmd_ls()` builtin (userland/
//! ring3-shell/shell.c), but as a real, separate ELF32 executable an
//! interactive shell can `run`/exec rather than something duplicated
//! inline. SYS_LIST_FILES needs no special capability (see kernel/
//! arch/x86/cpu/syscall.c - handle_list_files() has no capability
//! check at all), so this program works when launched via plain
//! SYS_EXEC, unlike cp.rs/rm.rs in this same directory.
//!
//! `#![no_std]`/`#![no_main]`: see ffi.rs's own header comment - no
//! heap, no OS-provided standard library, matching every other Rust
//! userland program in this project.

#![no_std]
#![no_main]

mod ffi;

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let msg = b"ls: internal error (panic)\n\0";
    unsafe {
        ffi::sys_write(msg.as_ptr());
    }
    loop {}
}

/// Matches kernel/arch/x86/cpu/syscall.c's own list_files_staging
/// buffer size (2048 bytes) - no point asking for more than the
/// kernel's own accumulator could ever have filled.
const LISTING_BUF_LEN: usize = 2048;
static mut LISTING_BUF: [u8; LISTING_BUF_LEN] = [0; LISTING_BUF_LEN];

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8, _envp: *const *const u8) -> i32 {
    // core::ptr::addr_of_mut!/addr_of! rather than LISTING_BUF.as_mut_
    // ptr()/.as_ptr(): the latter implicitly forms a Rust reference to
    // the static first (a real, if usually harmless in a single-
    // threaded program, soundness hazard the 2024 edition's own static_
    // mut_refs lint flags) before decaying it to a pointer; addr_of!
    // takes the address directly, without ever creating that
    // reference.
    unsafe {
        let buf_ptr = core::ptr::addr_of_mut!(LISTING_BUF) as *mut u8;
        let n = ffi::sys_list_files(buf_ptr, LISTING_BUF_LEN as i32 - 1);
        if n < 0 {
            ffi::sys_write(b"ls: no filesystem mounted, or listing too large\n\0".as_ptr());
            return 1;
        }
        *buf_ptr.add(n as usize) = 0; // NUL-terminate for sys_write()
        ffi::sys_write(buf_ptr);
    }
    0
}
