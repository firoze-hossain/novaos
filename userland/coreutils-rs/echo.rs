//! echo.rs - a genuine, standalone ring-3 `echo` program, written in
//! Rust (Phase 59).
//!
//! Prints argv[1..], space-separated, followed by a newline - the same
//! shape as the existing ring-3 shell's own inline `cmd_echo()`
//! builtin (userland/ring3-shell/shell.c), but as a real, separate
//! ELF32 executable. Needs no special capability (just SYS_WRITE),
//! so - like ls.rs - this works when launched via plain SYS_EXEC.
//!
//! `#![no_std]`/`#![no_main]`: see ffi.rs's own header comment.

#![no_std]
#![no_main]

mod ffi;

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let msg = b"echo: internal error (panic)\n\0";
    unsafe {
        ffi::sys_write(msg.as_ptr());
    }
    loop {}
}

/// The manual, no_std equivalent of CStr::from_ptr() - matches
/// userland/ping-rs/main.rs's own c_str_to_rust_str() exactly (see
/// that function's own comment: assumes valid UTF-8, true for every
/// argv this kernel's ELF loader passes through).
unsafe fn c_str_to_rust_str<'a>(ptr: *const u8) -> &'a str {
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    let slice = core::slice::from_raw_parts(ptr, len);
    core::str::from_utf8_unchecked(slice)
}

/// A fixed-size, no_std output buffer - same shape as ping-rs's own
/// Buf, sized generously for a shell command line (this kernel's own
/// LINE_BUF_SIZE in userland/ring3-shell/shell.c is 128).
struct Buf {
    data: [u8; 512],
    len: usize,
}

impl Buf {
    fn new() -> Self {
        Buf { data: [0; 512], len: 0 }
    }

    fn push_str(&mut self, s: &str) {
        for b in s.bytes() {
            if self.len < self.data.len() - 1 {
                self.data[self.len] = b;
                self.len += 1;
            }
        }
    }

    fn flush(&mut self) {
        self.data[self.len] = 0;
        unsafe {
            ffi::sys_write(self.data.as_ptr());
        }
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8, _envp: *const *const u8) -> i32 {
    let mut buf = Buf::new();
    for i in 1..argc {
        let arg = unsafe { c_str_to_rust_str(*argv.add(i as usize)) };
        buf.push_str(arg);
        if i < argc - 1 {
            buf.push_str(" ");
        }
    }
    buf.push_str("\n");
    buf.flush();
    0
}
