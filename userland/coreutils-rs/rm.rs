//! rm.rs - a genuine, standalone ring-3 `rm` program, written in Rust
//! (Phase 59) - the first `rm` this project has ever had, in any ring
//! or language.
//!
//! A single SYS_DELETE_FILE call against argv[1]. Like cp.rs in this
//! same directory, SYS_DELETE_FILE needs can_open_any_file (see
//! kernel/task/process.h) - a plain SYS_EXEC'd process has none of it,
//! so this program only works when the shell launches it via SYS_EXEC_
//! TRUSTED (see kernel/arch/x86/cpu/syscall.h's own comment on that
//! syscall, and userland/ring3-shell/shell.c's cmd_rm()).
//!
//! `#![no_std]`/`#![no_main]`: see ffi.rs's own header comment.

#![no_std]
#![no_main]

mod ffi;

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let msg = b"rm: internal error (panic)\n\0";
    unsafe {
        ffi::sys_write(msg.as_ptr());
    }
    loop {}
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8, _envp: *const *const u8) -> i32 {
    if argc < 2 {
        unsafe {
            ffi::sys_write(b"usage: rm FILE\n\0".as_ptr());
        }
        return 1;
    }

    // *argv.add(1) is argv[1]'s own original, already NUL-terminated C
    // string from the ELF loader's own stack setup - passed straight
    // through as the raw pointer sys_delete_file() expects, no
    // conversion needed (unlike cp.rs's src, nothing here ever needs
    // to know its length in Rust terms).
    let filename = unsafe { *argv.add(1) };

    let result = unsafe { ffi::sys_delete_file(filename) };
    if result < 0 {
        unsafe {
            ffi::sys_write(b"rm: cannot delete file (not found, or no write access)\n\0".as_ptr());
        }
        return 1;
    }

    0
}
