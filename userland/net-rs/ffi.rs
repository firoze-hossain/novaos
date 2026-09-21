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
}
