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

    /// Phase 89: writes a structured log entry into the kernel logging ring buffer.
    pub fn sys_klog_write(req: *const NovaKlogWrite) -> i32;

    /// Month 4: publishes supervisor service status snapshot to kernel table.
    pub fn sys_service_publish(status: *const NovaServiceStatus) -> i32;

    /// Month 4: retrieves any pending operator control action (restart, stop, start).
    pub fn sys_service_get_action(out: *mut NovaServiceAction) -> i32;
}

pub const NOVA_SVC_STATE_STOPPED: u32 = 0;
pub const NOVA_SVC_STATE_RUNNING: u32 = 1;
pub const NOVA_SVC_STATE_RESTARTING: u32 = 2;
pub const NOVA_SVC_STATE_CRASHED: u32 = 3;
pub const NOVA_SVC_STATE_BLOCKED: u32 = 4;

pub const NOVA_SVC_ACTION_NONE: u32 = 0;
pub const NOVA_SVC_ACTION_RESTART: u32 = 1;
pub const NOVA_SVC_ACTION_STOP: u32 = 2;
pub const NOVA_SVC_ACTION_START: u32 = 3;

pub const NOVA_SVC_MAX_SERVICES: usize = 16;
pub const NOVA_SVC_MAX_NAME: usize = 24;
pub const NOVA_SVC_MAX_PATH: usize = 16;
pub const NOVA_SVC_MAX_ARG: usize = 32;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NovaServiceInfo {
    pub name: [u8; NOVA_SVC_MAX_NAME],
    pub exec_path: [u8; NOVA_SVC_MAX_PATH],
    pub arg: [u8; NOVA_SVC_MAX_ARG],
    pub depends_on: [u8; NOVA_SVC_MAX_NAME],
    pub pid: i32,
    pub state: u32,
    pub restart_policy: u32,
    pub restart_count: u32,
    pub uptime_ticks: u32,
    pub trusted: u32,
    pub last_exit_code: i32,
    pub reserved: [u32; 3],
}

impl NovaServiceInfo {
    pub const fn empty() -> Self {
        NovaServiceInfo {
            name: [0u8; NOVA_SVC_MAX_NAME],
            exec_path: [0u8; NOVA_SVC_MAX_PATH],
            arg: [0u8; NOVA_SVC_MAX_ARG],
            depends_on: [0u8; NOVA_SVC_MAX_NAME],
            pid: -1,
            state: NOVA_SVC_STATE_STOPPED,
            restart_policy: 0,
            restart_count: 0,
            uptime_ticks: 0,
            trusted: 0,
            last_exit_code: 0,
            reserved: [0u32; 3],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NovaServiceStatus {
    pub count: u32,
    pub supervisor_pid: u32,
    pub supervisor_uptime_ticks: u32,
    pub generation: u32,
    pub pending_action: u32,
    pub pending_name: [u8; NOVA_SVC_MAX_NAME],
    pub services: [NovaServiceInfo; NOVA_SVC_MAX_SERVICES],
}

impl NovaServiceStatus {
    pub const fn empty() -> Self {
        NovaServiceStatus {
            count: 0,
            supervisor_pid: 1,
            supervisor_uptime_ticks: 0,
            generation: 0,
            pending_action: NOVA_SVC_ACTION_NONE,
            pending_name: [0u8; NOVA_SVC_MAX_NAME],
            services: [NovaServiceInfo::empty(); NOVA_SVC_MAX_SERVICES],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NovaServiceAction {
    pub action: u32,
    pub name: [u8; NOVA_SVC_MAX_NAME],
}

impl NovaServiceAction {
    pub const fn empty() -> Self {
        NovaServiceAction {
            action: NOVA_SVC_ACTION_NONE,
            name: [0u8; NOVA_SVC_MAX_NAME],
        }
    }
}

#[repr(C)]
pub struct NovaKlogWrite {
    pub severity: u32,
    pub facility: *const u8,
    pub msg: *const u8,
}
