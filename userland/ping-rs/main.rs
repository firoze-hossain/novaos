//! ping.rs - a genuine ring-3 `ping` program, written in Rust
//! (Phase 33).
//!
//! Parses a dotted-decimal IPv4 address from argv[1], sends an ICMP
//! Echo Request via the new SYS_PING_START syscall, and polls for the
//! reply via SYS_PING_POLL - a real, bounded ~3-second network
//! timeout, handled correctly: a syscall runs with interrupts
//! disabled for its entire duration (see syscall.h's comment on
//! SYS_PING_START/POLL), so the actual waiting happens here, in
//! ring-3, via the same SYS_YIELD-between-polls loop this project's
//! other blocking waits already use - not inside the kernel.
//!
//! `#![no_std]`: no heap allocation, no OS-provided standard library -
//! this links only against core (compiler_builtins is pulled in
//! automatically for integer operations the target's CPU can't do
//! natively). All string formatting here is done by hand into fixed
//! stack buffers, since there's no `alloc` and no `std::fmt` output
//! stream to write into.

#![no_std]
#![no_main]

mod ffi;

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let msg = b"ping: internal error (panic)\n\0";
    unsafe {
        ffi::sys_write(msg.as_ptr());
    }
    loop {}
}

/// A fixed-size, no_std string buffer for building output before a
/// single sys_write() call - there's no heap here to build a real
/// String with.
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
/// big-endian u32 matching what SYS_PING_START/ip_send() expect
/// elsewhere in this kernel. Returns None on any malformed input -
/// deliberately strict rather than guessing at a partial address.
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
/// the manual, no_std equivalent of what CStr::from_ptr() would do
/// with `std` available. Assumes valid UTF-8 (true for every argv
/// this kernel's ELF loader passes through, which only ever
/// originate from ASCII filenames/shell input).
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
    if argc < 2 {
        unsafe {
            ffi::sys_write(b"usage: ping HOST_IP\0".as_ptr());
        }
        return 1;
    }

    let host_str = unsafe { c_str_to_rust_str(*argv.add(1)) };

    let dest_ip = match parse_ipv4(host_str) {
        Some(ip) => ip,
        None => {
            let mut buf = Buf::new();
            buf.push_str("ping: '");
            buf.push_str(host_str);
            buf.push_str("' is not a valid dotted-decimal IPv4 address\n");
            buf.flush();
            return 1;
        }
    };

    let mut buf = Buf::new();
    buf.push_str("PING ");
    buf.push_str(host_str);
    buf.push_str("\n");
    buf.flush();

    unsafe {
        ffi::sys_ping_start(dest_ip);
    }

    let mut rtt: u32 = 0;
    loop {
        let result = unsafe { ffi::sys_ping_poll(&mut rtt as *mut u32) };
        if result == 1 {
            let mut buf = Buf::new();
            buf.push_str("Reply from ");
            buf.push_str(host_str);
            buf.push_str(": time=");
            buf.push_u32(rtt);
            buf.push_str(" ticks (~");
            buf.push_u32(rtt * 10);
            buf.push_str("ms)\n");
            buf.flush();
            return 0;
        }
        if result == -1 {
            let mut buf = Buf::new();
            buf.push_str("Request timed out (no reply from ");
            buf.push_str(host_str);
            buf.push_str(" after ~3s)\n");
            buf.flush();
            return 1;
        }
        unsafe {
            ffi::sys_yield();
        }
    }
}
