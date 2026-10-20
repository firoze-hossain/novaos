//! userland/novaupdate-rs/ffi.rs - FFI bindings for NovaOS ring-3 update service
//!
//! Provides bindings to the underlying NovaOS syscalls (userland/libc/syscall.c)
//! for console I/O, DNS resolution, Berkeley TCP sockets, and file operations.

#![allow(dead_code)]

extern "C" {
    /// Writes a NUL-terminated string to console and kernel debug log.
    pub fn sys_write(s: *const u8) -> i32;

    /// Voluntarily yields the remainder of the current quantum to the scheduler.
    pub fn sys_yield();

    /// Resolves `hostname` (NUL-terminated C string) via the kernel DNS resolver.
    /// Returns 1 on success (storing IPv4 into *out_ip in network byte order), or -1 on error.
    pub fn sys_dns_resolve(hostname: *const u8, out_ip: *mut u32) -> i32;

    /// Allocates an IPv4 TCP socket handle. Returns >= 0 on success, or -1.
    pub fn sys_socket() -> i32;

    /// Connects a TCP socket to `dest_ip` (packed IPv4) and `dest_port` (host byte order).
    /// Returns 0 on success, or -1 on failure.
    pub fn sys_connect(handle: i32, dest_ip: u32, dest_port: u16) -> i32;

    /// Writes `len` bytes from `buf` to the open socket/pipe handle.
    /// Returns bytes written, or -1 on failure.
    pub fn sys_write_handle(handle: i32, buf: *const u8, len: i32) -> i32;

    /// Reads up to `max_len` bytes from handle into `buf`.
    /// Returns bytes read (>0), 0 on EOF, -1 on error, or -2 if would block.
    pub fn sys_read(handle: i32, buf: *mut u8, max_len: i32) -> i32;

    /// Closes a previously opened socket, pipe, or file handle.
    pub fn sys_close(handle: i32);

    /// Opens an existing file by 8.3 name, returning a handle or -1.
    pub fn sys_open(filename: *const u8) -> i32;

    /// Atomically writes an entire file to disk under 8.3 name.
    /// Requires can_open_any_file capability (granted via SYS_EXEC_TRUSTED).
    pub fn sys_write_file(filename: *const u8, data: *const u8, size: u32) -> i32;

    /// Deletes a file by 8.3 name.
    pub fn sys_delete_file(filename: *const u8) -> i32;
}
