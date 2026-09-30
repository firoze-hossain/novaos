//! userland/novainit-rs/ffi.rs - FFI bindings to NovaOS's existing,
//! unmodified C syscall wrapper layer (userland/libc/syscall.c) - the
//! same "binding layer, not new functionality" approach every other
//! userland/*-rs/ffi.rs already established (Phase 33 onward). The
//! one genuinely new syscall this phase itself added, SYS_WAIT_
//! NONBLOCK, is the minimum real kernel surface a service supervisor
//! actually needs and didn't already have - everything else here is
//! syscalls that already existed and already worked.

#![allow(dead_code)]

extern "C" {
    /// Writes a NUL-terminated string to the console/debug log.
    pub fn sys_write(s: *const u8) -> i32;

    /// Yields the remaining timeslice back to the scheduler.
    pub fn sys_yield();

    /// Opens a file for reading by name (capability-gated - see
    /// kernel/arch/x86/cpu/syscall.h's own SYS_OPEN comment). Returns
    /// a handle >= 0, or -1 if denied/not found.
    pub fn sys_open(filename: *const u8) -> i32;

    /// Reads up to `max_len` bytes from `handle` into `buf`, returning
    /// bytes actually read, 0 at real EOF, or -1 on error.
    pub fn sys_read(handle: i32, buf: *mut u8, max_len: i32) -> i32;

    /// Closes a handle previously returned by sys_open().
    pub fn sys_close(handle: i32);

    /// Loads and runs `path` as a new child process, with `argv[0..
    /// argc]` as its own argv. Returns the new pid, or -1 on failure
    /// (not found, or a real ELF-loading error).
    pub fn sys_exec(path: *const u8, argv: *const *const u8, argc: i32) -> i32;

    /// Phase 77: like sys_exec() above, except the new process
    /// inherits THIS process's own can_open_any_file/can_spawn
    /// grants, if it has any (see kernel/task/process.c's
    /// process_exec_trusted_env() for the exact, real delegation
    /// rule - a caller with neither capability delegates nothing at
    /// all, making this identical to plain sys_exec() for it). Used
    /// by start_service() for exactly the services this file's own
    /// SERVICES.CFG marks `trusted` - see that function's own comment.
    pub fn sys_exec_trusted(path: *const u8, argv: *const *const u8, argc: i32) -> i32;

    /// Blocks until `pid` terminates, then returns its exit code (or
    /// -1 immediately if no such process exists at all).
    pub fn sys_wait(pid: i32) -> i32;

    /// Phase 71's own new syscall - checks `pid`'s own state exactly
    /// once, never blocks. Returns -2 (no such process), -1 (exists,
    /// still running), or 0 (terminated, with `*out_exit_code`
    /// written) - see kernel/arch/x86/cpu/syscall.h's own SYS_WAIT_
    /// NONBLOCK doc comment for the full contract this is a thin
    /// wrapper over.
    pub fn sys_wait_nonblock(pid: i32, out_exit_code: *mut i32) -> i32;
}
