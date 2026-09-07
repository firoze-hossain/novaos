//! FFI bindings to NovaOS's existing, unmodified C syscall wrapper
//! layer (userland/libc/syscall.c). These functions are already
//! implemented, tested, and in use by every other userland program
//! (cat.c, shell.c, gui.c) - this file only declares their C ABI so
//! Rust code can call them directly. No syscall logic is
//! reimplemented here; this is a binding layer, not new
//! functionality.

#![allow(dead_code)]

extern "C" {
    /// userland/libc/syscall.c's sys_write() - writes a NUL-terminated
    /// string to the console (and, on the kernel side, the debug log).
    pub fn sys_write(s: *const u8) -> i32;

    /// Yields the remaining timeslice back to the scheduler.
    pub fn sys_yield();

    /// Starts a non-blocking ping to `dest_ip` (packed big-endian
    /// IPv4, matching every other IP address this kernel passes
    /// around). Returns immediately.
    pub fn sys_ping_start(dest_ip: u32);

    /// Polls the outstanding ping started by sys_ping_start(). Returns
    /// 1 if a reply arrived (RTT, in timer ticks, written to
    /// `out_rtt`), 0 if still waiting, -1 if the ~3s deadline passed
    /// with no reply.
    pub fn sys_ping_poll(out_rtt: *mut u32) -> i32;
}
