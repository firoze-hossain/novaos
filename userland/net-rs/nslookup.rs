//! nslookup.rs - a genuine, standalone ring-3 `nslookup` program,
//! written in Rust (Phase 60) - the first `nslookup` this project has
//! ever had, in any ring or language. Closes the release-readiness
//! doc's own 2.1 row for this one command.
//!
//! A single SYS_DNS_RESOLVE call against argv[1], printing a real-
//! nslookup-shaped result (a "Server:"/"Address:" header naming this
//! kernel's one configured resolver, then the resolved name/address)
//! rather than inventing a novel output format - the same "match the
//! familiar tool" instruction the release-readiness doc gives for
//! coreutils naming/flags, applied here to output shape too. Needs no
//! special capability (a DNS lookup touches no local file and reaches
//! a fixed address, not one this program chooses - see kernel/arch/
//! x86/cpu/syscall.h's own comment on SYS_DNS_RESOLVE), so this
//! program works when launched via plain SYS_EXEC, unlike tftp.rs in
//! this same directory.
//!
//! `#![no_std]`/`#![no_main]`: see ffi.rs's own header comment - no
//! heap, no OS-provided standard library, matching every other Rust
//! userland program in this project. All string/number formatting is
//! done by hand into a fixed stack buffer, the same `Buf` pattern
//! userland/ping-rs/main.rs and userland/coreutils-rs/echo.rs already
//! use, since there's no `alloc` and no `std::fmt` output stream to
//! write into.

#![no_std]
#![no_main]

mod ffi;

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let msg = b"nslookup: internal error (panic)\n\0";
    unsafe {
        ffi::sys_write(msg.as_ptr());
    }
    loop {}
}

/// This kernel's one fixed, configured DNS resolver (NET_DNS_SERVER_IP
/// in kernel/net/net.h - 10.0.2.3, QEMU SLIRP's built-in DNS proxy).
/// There's no way to ask this kernel to use a different one (no
/// resolv.conf, no DHCP-provided resolver - see net.h's own comment),
/// so hard-coding the same address here for display purposes only
/// (never sent anywhere - SYS_DNS_RESOLVE itself already knows it)
/// matches this kernel's "one fixed static config" scope everywhere
/// else, not a new limitation this program introduces.
const DNS_SERVER_OCTETS: [u8; 4] = [10, 0, 2, 3];

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

    fn push_u32_decimal(&mut self, mut n: u32) {
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

    /// Formats a packed big-endian IPv4 address as dotted-decimal
    /// ("10.0.2.3") - the inverse of ping.rs's own parse_ipv4().
    fn push_ipv4(&mut self, ip: u32) {
        self.push_u32_decimal((ip >> 24) & 0xFF);
        self.push_str(".");
        self.push_u32_decimal((ip >> 16) & 0xFF);
        self.push_str(".");
        self.push_u32_decimal((ip >> 8) & 0xFF);
        self.push_str(".");
        self.push_u32_decimal(ip & 0xFF);
    }

    fn push_octets(&mut self, octets: &[u8; 4]) {
        self.push_u32_decimal(octets[0] as u32);
        self.push_str(".");
        self.push_u32_decimal(octets[1] as u32);
        self.push_str(".");
        self.push_u32_decimal(octets[2] as u32);
        self.push_str(".");
        self.push_u32_decimal(octets[3] as u32);
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

/// Reads a NUL-terminated C string from a raw pointer into a &str -
/// the manual, no_std equivalent of what CStr::from_ptr() would do
/// with `std` available (see userland/ping-rs/main.rs's own copy of
/// this same helper).
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
            ffi::sys_write(b"usage: nslookup HOSTNAME\n\0".as_ptr());
        }
        return 1;
    }

    // hostname's underlying bytes are argv[1]'s own original, already
    // NUL-terminated C string (c_str_to_rust_str() only trims the
    // slice's *length* at the NUL, it doesn't copy) - so
    // hostname.as_ptr() is a valid C string pointer here, not just a
    // &str, the same reasoning userland/coreutils-rs/cp.rs's src
    // already documents.
    let hostname = unsafe { c_str_to_rust_str(*argv.add(1)) };

    {
        let mut buf = Buf::new();
        buf.push_str("Server:  ");
        buf.push_octets(&DNS_SERVER_OCTETS);
        buf.push_str("\nAddress: ");
        buf.push_octets(&DNS_SERVER_OCTETS);
        buf.push_str("#53\n\n");
        buf.flush();
    }

    let mut resolved_ip: u32 = 0;
    let result = unsafe { ffi::sys_dns_resolve(hostname.as_ptr(), &mut resolved_ip as *mut u32) };

    if result == 1 {
        let mut buf = Buf::new();
        buf.push_str("Name:    ");
        buf.push_str(hostname);
        buf.push_str("\nAddress: ");
        buf.push_ipv4(resolved_ip);
        buf.push_str("\n");
        buf.flush();
        0
    } else {
        let mut buf = Buf::new();
        buf.push_str("** server can't find ");
        buf.push_str(hostname);
        buf.push_str(": NXDOMAIN (or timed out after ~3s)\n");
        buf.flush();
        1
    }
}
