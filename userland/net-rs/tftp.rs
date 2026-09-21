//! tftp.rs - a genuine, standalone ring-3 `tftp` program, written in
//! Rust (Phase 60) - the first `tftp` this project has ever had, in
//! any ring or language. Closes the release-readiness doc's own 2.1
//! row for this one command.
//!
//! Parses a dotted-decimal server IPv4 address from argv[1] (the same
//! parse_ipv4() shape userland/ping-rs/main.rs already established -
//! duplicated here rather than shared, the same "small enough to
//! copy, no shared crate between these small no_std programs" choice
//! userland/coreutils-rs/*.rs already makes for c_str_to_rust_str()),
//! then fetches argv[2] (the remote filename) via a single
//! SYS_TFTP_FETCH call, saving it locally as argv[3] if given or
//! argv[2] again if not.
//!
//! SYS_TFTP_FETCH creates/overwrites a local file, so - exactly like
//! userland/coreutils-rs/cp.rs and rm.rs - it needs the calling
//! process's own can_open_any_file capability (see kernel/task/
//! process.h), which a plain SYS_EXEC'd process never has. This
//! program only actually works when the shell launches it via
//! SYS_EXEC_TRUSTED (see kernel/arch/x86/cpu/syscall.h's own comment
//! on that syscall, and userland/ring3-shell/shell.c's cmd_tftp())
//! rather than plain `run`.
//!
//! `#![no_std]`/`#![no_main]`: see ffi.rs's own header comment.

#![no_std]
#![no_main]

mod ffi;

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let msg = b"tftp: internal error (panic)\n\0";
    unsafe {
        ffi::sys_write(msg.as_ptr());
    }
    loop {}
}

struct Buf {
    data: [u8; 256],
    len: usize,
}

impl Buf {
    fn new() -> Self {
        Buf {
            data: [0; 256],
            len: 0,
        }
    }

    fn push_str(&mut self, s: &str) {
        for b in s.bytes() {
            if self.len < self.data.len() - 1 {
                self.data[self.len] = b;
                self.len += 1;
            }
        }
    }

    fn push_u32(&mut self, mut n: u32) {
        if n == 0 {
            self.push_str("0");
            return;
        }
        let mut digits = [0u8; 10];
        let mut count = 0;
        while n > 0 {
            digits[count] = b'0' + (n % 10) as u8;
            n /= 10;
            count += 1;
        }
        while count > 0 {
            count -= 1;
            if self.len < self.data.len() - 1 {
                self.data[self.len] = digits[count];
                self.len += 1;
            }
        }
    }

    /// NUL-terminates and writes the buffer via sys_write(). Must be
    /// the last thing done with this buffer, since sys_write() needs
    /// a C string.
    fn flush(&mut self) {
        self.data[self.len] = 0;
        unsafe {
            ffi::sys_write(self.data.as_ptr());
        }
    }
}

/// Parses a dotted-decimal IPv4 address ("10.0.2.2") into a packed,
/// big-endian u32 - a direct copy of userland/ping-rs/main.rs's own
/// parse_ipv4(), same strict-rather-than-guessing behavior on
/// malformed input.
fn parse_ipv4(s: &str) -> Option<u32> {
    let mut octets = [0u32; 4];
    let mut count = 0;
    for part in s.split('.') {
        if count >= 4 {
            return None;
        }
        let mut value: u32 = 0;
        if part.is_empty() {
            return None;
        }
        for c in part.bytes() {
            if !c.is_ascii_digit() {
                return None;
            }
            value = value * 10 + (c - b'0') as u32;
            if value > 255 {
                return None;
            }
        }
        octets[count] = value;
        count += 1;
    }
    if count != 4 {
        return None;
    }
    Some((octets[0] << 24) | (octets[1] << 16) | (octets[2] << 8) | octets[3])
}

/// Reads a NUL-terminated C string from a raw pointer into a &str -
/// see userland/ping-rs/main.rs's own copy of this same helper.
unsafe fn c_str_to_rust_str<'a>(ptr: *const u8) -> &'a str {
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    let slice = core::slice::from_raw_parts(ptr, len);
    core::str::from_utf8_unchecked(slice)
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8, _envp: *const *const u8) -> i32 {
    if argc < 3 {
        unsafe {
            ffi::sys_write(b"usage: tftp SERVER_IP REMOTE_FILE [LOCAL_FILE]\n\0".as_ptr());
        }
        return 1;
    }

    let server_str = unsafe { c_str_to_rust_str(*argv.add(1)) };
    let server_ip = match parse_ipv4(server_str) {
        Some(ip) => ip,
        None => {
            let mut buf = Buf::new();
            buf.push_str("tftp: '");
            buf.push_str(server_str);
            buf.push_str("' is not a valid dotted-decimal IPv4 address\n");
            buf.flush();
            return 1;
        }
    };

    // remote_ptr/local_ptr are argv[2]/argv[3]'s own original,
    // already NUL-terminated C string pointers from the ELF loader's
    // own stack setup - passed straight through to sys_tftp_fetch(),
    // no conversion needed (the same reasoning userland/coreutils-rs/
    // rm.rs's filename already documents). local_ptr defaults to the
    // same pointer as remote_ptr when argv[3] wasn't given - saving
    // under the same name it was fetched with, the ordinary case.
    let remote_ptr = unsafe { *argv.add(2) };
    let local_ptr = if argc >= 4 {
        unsafe { *argv.add(3) }
    } else {
        remote_ptr
    };
    let remote_str = unsafe { c_str_to_rust_str(remote_ptr) };

    {
        let mut buf = Buf::new();
        buf.push_str("Fetching '");
        buf.push_str(remote_str);
        buf.push_str("' via TFTP...\n");
        buf.flush();
    }

    let n = unsafe { ffi::sys_tftp_fetch(server_ip, remote_ptr, local_ptr) };

    if n < 0 {
        unsafe {
            ffi::sys_write(
                b"tftp: fetch failed (timeout, TFTP error, file too large, or no write access)\n\0"
                    .as_ptr(),
            );
        }
        return 1;
    }

    let mut buf = Buf::new();
    buf.push_str("Received ");
    buf.push_u32(n as u32);
    buf.push_str(" bytes\n");
    buf.flush();
    0
}
