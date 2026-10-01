//! FFI bindings to NovaOS's existing, unmodified C syscall wrapper
//! layer (userland/libc/syscall.c) - the same "binding layer, not new
//! functionality" convention userland/ping-rs/ffi.rs and
//! userland/coreutils-rs/ffi.rs already establish, just for this
//! directory's own two programs (nslookup.rs, tftp.rs), Phase 60's
//! answer to the release-readiness doc's 2.1 row ("nslookup, tftp ...
//! aren't reachable from the ring-3 shell yet").

#![allow(dead_code)]

extern "C" {
    /// userland/libc/syscall.c's sys_write() - writes a NUL-terminated
    /// string to the console (and, on the kernel side, the debug log).
    pub fn sys_write(s: *const u8) -> i32;

    /// Yields the remaining timeslice back to the scheduler. Not
    /// needed by either program in this directory today (both new
    /// syscalls below are blocking, single-call operations - see
    /// kernel/arch/x86/cpu/syscall.h's own comment on why
    /// SYS_DNS_RESOLVE/SYS_TFTP_FETCH didn't need ping's start/poll
    /// split), but declared for parity with ffi.rs's siblings and in
    /// case a future addition to this directory needs it.
    pub fn sys_yield();

    /// Resolves `hostname` (NUL-terminated C string) via this
    /// kernel's one configured DNS resolver. Returns 1 with `*out_ip`
    /// filled in (packed big-endian IPv4, matching every other IP
    /// address this kernel passes around) on success, -1 on failure
    /// (NXDOMAIN, malformed response, or a ~3s timeout).
    pub fn sys_dns_resolve(hostname: *const u8, out_ip: *mut u32) -> i32;

    /// Fetches `remote_filename` from `server_ip` over TFTP and saves
    /// it as `local_filename` on this kernel's own mounted
    /// filesystem. Needs the calling process's own can_open_any_file
    /// capability (see kernel/task/process.h) - a plain SYS_EXEC'd
    /// process has none, so this only succeeds when launched via
    /// sys_exec_trusted() (see userland/ring3-shell/shell.c's
    /// cmd_tftp()). Returns the number of bytes fetched and written,
    /// or -1 on failure.
    pub fn sys_tftp_fetch(
        server_ip: u32,
        remote_filename: *const u8,
        local_filename: *const u8,
    ) -> i32;

    /// Phase 78: real, multi-socket UDP - kernel/rust/udp.rs's own
    /// module comment has the full design. See userland/libc/include/
    /// novasys.h's own identical declarations (this file's own
    /// established "bind directly to the unmodified C syscall layer"
    /// convention, not a reimplementation) for the full contract of
    /// each.
    pub fn sys_socket_udp() -> i32;
    pub fn sys_bind(handle: i32, port: u16) -> i32;
    pub fn sys_connect(handle: i32, dest_ip: u32, dest_port: u16) -> i32;
    pub fn sys_sendto(handle: i32, addr: *const NovaUdpAddr, buf: *const u8, len: u32) -> i32;
    pub fn sys_recvfrom(
        handle: i32,
        buf: *mut u8,
        max_len: u32,
        out_addr: *mut NovaUdpAddr,
    ) -> i32;
    pub fn sys_read(handle: i32, buf: *mut u8, max_len: i32) -> i32;
    pub fn sys_write_handle(handle: i32, buf: *const u8, len: i32) -> i32;
    pub fn sys_close(handle: i32);
}

/// The exact 6-byte layout kernel/arch/x86/cpu/syscall.c's own
/// nova_udp_addr_t / userland/libc/include/novasys.h's own C
/// equivalent both use - `#[repr(C)]`, not Rust's own default layout,
/// since this is read/written directly across the syscall boundary at
/// a raw pointer, byte-for-byte (see this project's own established
/// convention for this, e.g. kernel/rust/dynlink.rs's own structs
/// mirroring kernel/task/elf.c's). `ip` is host byte order, matching
/// every other IP address this kernel passes around.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NovaUdpAddr {
    pub ip: u32,
    pub port: u16,
}
