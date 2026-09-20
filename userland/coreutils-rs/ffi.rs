//! FFI bindings to NovaOS's existing, unmodified C syscall wrapper
//! layer (userland/libc/syscall.c) - the same "binding layer, not new
//! functionality" approach userland/ping-rs/ffi.rs already established
//! (Phase 33). Shared by every program in this directory (ls.rs,
//! echo.rs, cp.rs, rm.rs): each is compiled as its own separate crate
//! (`rustc <name>.rs --crate-type bin`, no Cargo), but `mod ffi;` in
//! each resolves to this one file since all five live in the same
//! directory - not a "no shared bridge header" violation, since that
//! convention is specifically about the C<->Rust boundary; this file
//! only declares bindings already used identically by ping-rs.
//!
//! A superset of what any single program here needs - each program's
//! own `mod ffi;` still pulls in the whole file, but `#[allow(dead_
//! code)]` below means an unused declaration is silent rather than a
//! warning, matching ping-rs/ffi.rs's own choice.

#![allow(dead_code)]

extern "C" {
    /// userland/libc/syscall.c's sys_write() - writes a NUL-terminated
    /// string to the console (and, on the kernel side, the debug log).
    pub fn sys_write(s: *const u8) -> i32;

    /// Opens a file by (8.3) name, returning a handle or -1. Gated by
    /// this process's own file capability (see kernel/task/process.h's
    /// can_open_any_file/allowed_files[]) - see kernel/arch/x86/cpu/
    /// syscall.h's own comment on SYS_OPEN.
    pub fn sys_open(filename: *const u8) -> i32;

    /// Reads up to `max_len` bytes from `handle` into `buf`, returning
    /// bytes actually read, 0 at real EOF, or -1 on error/invalid
    /// handle.
    pub fn sys_read(handle: i32, buf: *mut u8, max_len: i32) -> i32;

    /// Closes a handle previously returned by sys_open() (or sys_pipe()/
    /// sys_socket()).
    pub fn sys_close(handle: i32);

    /// Lists every file on the mounted filesystem into `buf` as
    /// "NAME SIZE\n" lines (see kernel/arch/x86/cpu/syscall.c's
    /// handle_list_files()), returning the number of bytes written, or
    /// -1 if nothing is mounted or the listing doesn't fit in
    /// `buf_size`. Not capability-gated.
    pub fn sys_list_files(buf: *mut u8, buf_size: i32) -> i32;

    /// Creates (create-only - fails if `filename` already exists)
    /// `filename` with exactly `data[..size]` as its whole contents in
    /// one call - there is no incremental/append write on this kernel.
    /// Gated by can_open_any_file (see process.h) - a plain SYS_EXEC'd
    /// process has none of it; SYS_EXEC_TRUSTED (see sys_exec_trusted()
    /// below) is what lets a program like cp/rm genuinely need this.
    /// Returns >= 0 on success, -1 on failure (denied, or the name
    /// already exists).
    pub fn sys_write_file(filename: *const u8, data: *const u8, size: u32) -> i32;

    /// Deletes `filename`. Same can_open_any_file gate as
    /// sys_write_file(). Returns >= 0 on success, -1 on failure (denied,
    /// or not found).
    pub fn sys_delete_file(filename: *const u8) -> i32;

    /// Yields the remaining timeslice back to the scheduler.
    pub fn sys_yield();
}
