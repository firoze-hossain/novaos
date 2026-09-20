//! cp.rs - a genuine, standalone ring-3 `cp` program, written in Rust
//! (Phase 59) - the first `cp` this project has ever had, in any ring
//! or language.
//!
//! Reads argv[1] (source) in full via SYS_OPEN/SYS_READ into a fixed,
//! bounded buffer, then writes it out as argv[2] (destination) in one
//! SYS_WRITE_FILE call - this kernel has no incremental/append file
//! write (see ffi.rs's own comment on sys_write_file()), so a whole-
//! file buffer-then-write is the only shape available, not a
//! simplification chosen here. SYS_WRITE_FILE (and, for reading an
//! arbitrary source the user names at the prompt rather than one
//! already on this process's own capability list, SYS_OPEN too) both
//! need can_open_any_file (see kernel/task/process.h) - a plain SYS_
//! EXEC'd process has none of it, so this program only works when the
//! shell launches it via SYS_EXEC_TRUSTED (see kernel/arch/x86/cpu/
//! syscall.h's own comment on that syscall, and userland/ring3-shell/
//! shell.c's cmd_cp()) rather than plain `run`.
//!
//! `#![no_std]`/`#![no_main]`: see ffi.rs's own header comment.

#![no_std]
#![no_main]

mod ffi;

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let msg = b"cp: internal error (panic)\n\0";
    unsafe {
        ffi::sys_write(msg.as_ptr());
    }
    loop {}
}

unsafe fn c_str_to_rust_str<'a>(ptr: *const u8) -> &'a str {
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    let slice = core::slice::from_raw_parts(ptr, len);
    core::str::from_utf8_unchecked(slice)
}

/// 256KB - generous for this kernel's own real-world file sizes today
/// (every existing fixture, package payload, and journaled/ext2 test
/// file is well under this), and small enough to fit comfortably
/// alongside this program's own tiny code/stack in the 2MB process_
/// exec_internal() reserves for loading an ELF - see kernel/task/
/// process.c's own elf_buffer comment for that number's own reasoning.
/// A real cp would stream in fixed-size chunks instead of buffering
/// the whole file - an honest, stated scope limit, not an oversight,
/// matching this kernel's own SYS_WRITE_FILE contract (see ffi.rs) of
/// needing the whole file in memory at once regardless.
const COPY_BUF_LEN: usize = 256 * 1024;
static mut COPY_BUF: [u8; COPY_BUF_LEN] = [0; COPY_BUF_LEN];

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8, _envp: *const *const u8) -> i32 {
    if argc < 3 {
        unsafe {
            ffi::sys_write(b"usage: cp SOURCE DEST\n\0".as_ptr());
        }
        return 1;
    }

    let src = unsafe { c_str_to_rust_str(*argv.add(1)) };
    let dst_ptr = unsafe { *argv.add(2) };

    // src's underlying bytes are argv[1]'s own original, already NUL-
    // terminated C string (c_str_to_rust_str() only trims the slice's
    // *length* at the NUL, it doesn't copy) - so src.as_ptr() is a
    // valid C string pointer here, not just a &str.
    let fd = unsafe { ffi::sys_open(src.as_ptr()) };
    if fd < 0 {
        unsafe {
            ffi::sys_write(b"cp: cannot open source file\n\0".as_ptr());
        }
        return 1;
    }

    // core::ptr::addr_of_mut! rather than COPY_BUF.as_mut_ptr(): see
    // ls.rs's own comment on the same choice - takes the static's
    // address directly, without going through an implicit Rust
    // reference to it first.
    let buf_ptr = core::ptr::addr_of_mut!(COPY_BUF) as *mut u8;

    let mut total: usize = 0;
    loop {
        let remaining = COPY_BUF_LEN - total;
        if remaining == 0 {
            unsafe {
                ffi::sys_close(fd);
                ffi::sys_write(
                    b"cp: source file too large for this program's fixed copy buffer\n\0"
                        .as_ptr(),
                );
            }
            return 1;
        }
        let n = unsafe { ffi::sys_read(fd, buf_ptr.add(total), remaining as i32) };
        if n < 0 {
            unsafe {
                ffi::sys_close(fd);
                ffi::sys_write(b"cp: read error on source file\n\0".as_ptr());
            }
            return 1;
        }
        if n == 0 {
            break; // real EOF
        }
        total += n as usize;
    }
    unsafe {
        ffi::sys_close(fd);
    }

    let written = unsafe { ffi::sys_write_file(dst_ptr, buf_ptr, total as u32) };
    if written < 0 {
        unsafe {
            ffi::sys_write(
                b"cp: cannot write destination file (already exists, or no write access)\n\0"
                    .as_ptr(),
            );
        }
        return 1;
    }

    0
}
