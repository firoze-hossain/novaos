//! kernel/rust/services.rs - Month 4: Service Manager & Supervisor live status.
//!
//! Provides the kernel-side shared service state table for the userland service supervisor
//! (`novainit-rs`, PID 1) and administrative clients (`settings-rs` Service Manager panel).
//!
//! # Architecture & Guarantees
//!
//! 1. Zero Heap Allocation & Memory Bounded:
//!    Fixed static table of up to 16 services (`NOVA_SVC_MAX_SERVICES`). All operations
//!    execute in bounded O(1) time without dynamic heap allocation.
//!
//! 2. Concurrency Safety:
//!    Guarded by `SpinLock`, safe for concurrent queries and updates across SMP cores.
//!
//! 3. Privilege Gating:
//!    Only supervisor (PID 1) or privileged processes (caller_pid <= 2 or root) can publish
//!    service supervision state updates via `SYS_SERVICE_STATUS` (`NOVA_SVC_OP_PUBLISH`).
//!    Any userland process can query live service telemetry (`NOVA_SVC_OP_QUERY`).
//!
//! 4. Bi-directional Control Channel:
//!    Administrative panels can request service restarts, stops, or starts
//!    (`NOVA_SVC_OP_CONTROL`), which are recorded in the table for `novainit` to execute
//!    on its next supervision pass (`NOVA_SVC_OP_GET_ACTION`).

#![cfg_attr(not(test), no_std)]

#[cfg(not(test))]
use crate::spinlock::SpinLock;

#[cfg(test)]
pub struct SpinLock<T>(std::sync::Mutex<T>);

#[cfg(test)]
impl<T> SpinLock<T> {
    pub const fn new(val: T) -> Self {
        Self(std::sync::Mutex::new(val))
    }
    pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
        self.0.lock().unwrap()
    }
}

pub const NOVA_SYS_SERVICE_STATUS: u32 = 82;

pub const NOVA_SVC_OP_QUERY: u32 = 1;
pub const NOVA_SVC_OP_PUBLISH: u32 = 2;
pub const NOVA_SVC_OP_CONTROL: u32 = 3;
pub const NOVA_SVC_OP_GET_ACTION: u32 = 4;

pub const NOVA_SVC_STATE_STOPPED: u32 = 0;
pub const NOVA_SVC_STATE_RUNNING: u32 = 1;
pub const NOVA_SVC_STATE_RESTARTING: u32 = 2;
pub const NOVA_SVC_STATE_CRASHED: u32 = 3;
pub const NOVA_SVC_STATE_BLOCKED: u32 = 4;

pub const NOVA_SVC_POLICY_NEVER: u32 = 0;
pub const NOVA_SVC_POLICY_ON_CRASH: u32 = 1;
pub const NOVA_SVC_POLICY_ALWAYS: u32 = 2;

pub const NOVA_SVC_ACTION_NONE: u32 = 0;
pub const NOVA_SVC_ACTION_RESTART: u32 = 1;
pub const NOVA_SVC_ACTION_STOP: u32 = 2;
pub const NOVA_SVC_ACTION_START: u32 = 3;

pub const NOVA_SVC_MAX_SERVICES: usize = 16;
pub const NOVA_SVC_MAX_NAME: usize = 24;
pub const NOVA_SVC_MAX_PATH: usize = 16;
pub const NOVA_SVC_MAX_ARG: usize = 32;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceInfo {
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

impl ServiceInfo {
    pub const fn empty() -> Self {
        ServiceInfo {
            name: [0u8; NOVA_SVC_MAX_NAME],
            exec_path: [0u8; NOVA_SVC_MAX_PATH],
            arg: [0u8; NOVA_SVC_MAX_ARG],
            depends_on: [0u8; NOVA_SVC_MAX_NAME],
            pid: -1,
            state: NOVA_SVC_STATE_STOPPED,
            restart_policy: NOVA_SVC_POLICY_NEVER,
            restart_count: 0,
            uptime_ticks: 0,
            trusted: 0,
            last_exit_code: 0,
            reserved: [0u32; 3],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceStatusTable {
    pub count: u32,
    pub supervisor_pid: u32,
    pub supervisor_uptime_ticks: u32,
    pub generation: u32,
    pub pending_action: u32,
    pub pending_name: [u8; NOVA_SVC_MAX_NAME],
    pub services: [ServiceInfo; NOVA_SVC_MAX_SERVICES],
}

impl ServiceStatusTable {
    pub const fn new() -> Self {
        ServiceStatusTable {
            count: 0,
            supervisor_pid: 1,
            supervisor_uptime_ticks: 0,
            generation: 0,
            pending_action: NOVA_SVC_ACTION_NONE,
            pending_name: [0u8; NOVA_SVC_MAX_NAME],
            services: [ServiceInfo::empty(); NOVA_SVC_MAX_SERVICES],
        }
    }

    pub fn set_pending_action(&mut self, action: u32, name: &[u8]) {
        self.pending_action = action;
        self.pending_name = [0u8; NOVA_SVC_MAX_NAME];
        let n = name.len().min(NOVA_SVC_MAX_NAME - 1);
        self.pending_name[..n].copy_from_slice(&name[..n]);
    }

    pub fn take_pending_action(&mut self, out_name: &mut [u8]) -> u32 {
        let action = self.pending_action;
        let n = out_name.len().min(NOVA_SVC_MAX_NAME);
        out_name[..n].copy_from_slice(&self.pending_name[..n]);
        self.pending_action = NOVA_SVC_ACTION_NONE;
        self.pending_name = [0u8; NOVA_SVC_MAX_NAME];
        action
    }
}

static SERVICE_TABLE: SpinLock<ServiceStatusTable> = SpinLock::new(ServiceStatusTable::new());

/// Publishes a service status update from `novainit` (or root task).
/// Preserves any pending operator control actions.
#[no_mangle]
pub extern "C" fn rust_service_status_publish(
    status: *const ServiceStatusTable,
    caller_pid: u32,
) -> i32 {
    if status.is_null() {
        return -14; // -EFAULT
    }
    // Allow supervisor (pid 1 or 2 during boot) or root (pid 0)
    if caller_pid > 2 {
        return -1; // -EPERM
    }

    let input = unsafe { &*status };
    let mut table = SERVICE_TABLE.lock();

    let saved_action = table.pending_action;
    let saved_name = table.pending_name;

    let count = (input.count as usize).min(NOVA_SVC_MAX_SERVICES);
    table.count = count as u32;
    table.supervisor_pid = input.supervisor_pid;
    table.supervisor_uptime_ticks = input.supervisor_uptime_ticks;
    table.generation = table.generation.wrapping_add(1);

    for i in 0..count {
        table.services[i] = input.services[i];
    }
    for i in count..NOVA_SVC_MAX_SERVICES {
        table.services[i] = ServiceInfo::empty();
    }

    // Preserve pending operator action if supervisor hasn't consumed it yet
    if saved_action != NOVA_SVC_ACTION_NONE {
        table.pending_action = saved_action;
        table.pending_name = saved_name;
    }

    0
}

/// Queries the live service supervisor status snapshot.
#[no_mangle]
pub extern "C" fn rust_service_status_query(out_status: *mut ServiceStatusTable) -> i32 {
    if out_status.is_null() {
        return -14; // -EFAULT
    }
    let table = SERVICE_TABLE.lock();
    unsafe {
        *out_status = *table;
    }
    0
}

/// Dispatches an administrative service control action (restart, stop, start).
#[no_mangle]
pub extern "C" fn rust_service_status_control(
    action: u32,
    name_ptr: *const u8,
    name_len: u32,
) -> i32 {
    if action > NOVA_SVC_ACTION_START {
        return -22; // -EINVAL
    }
    let mut table = SERVICE_TABLE.lock();
    if name_ptr.is_null() || name_len == 0 {
        table.pending_action = action;
        table.pending_name = [0u8; NOVA_SVC_MAX_NAME];
    } else {
        let slice = unsafe { core::slice::from_raw_parts(name_ptr, (name_len as usize).min(NOVA_SVC_MAX_NAME)) };
        table.set_pending_action(action, slice);
    }
    0
}

/// Retrieves and clears any pending administrative action. Called by `novainit`.
#[no_mangle]
pub extern "C" fn rust_service_status_get_action(
    out_action: *mut u32,
    out_name: *mut u8,
    max_name_len: u32,
) -> i32 {
    if out_action.is_null() || out_name.is_null() {
        return -14; // -EFAULT
    }
    let mut table = SERVICE_TABLE.lock();
    let name_buf = unsafe { core::slice::from_raw_parts_mut(out_name, max_name_len as usize) };
    let action = table.take_pending_action(name_buf);
    unsafe {
        *out_action = action;
    }
    0
}

/// In-kernel self-test verifying service status publish, query, control, and action retrieval.
#[no_mangle]
pub extern "C" fn rust_service_status_selftest() -> i32 {
    let mut status = ServiceStatusTable::new();
    status.count = 2;
    status.supervisor_pid = 1;
    status.supervisor_uptime_ticks = 4200;

    let s0 = &mut status.services[0];
    let n0 = b"network-check";
    s0.name[..n0.len()].copy_from_slice(n0);
    let p0 = b"PING.ELF";
    s0.exec_path[..p0.len()].copy_from_slice(p0);
    let a0 = b"10.0.2.2";
    s0.arg[..a0.len()].copy_from_slice(a0);
    s0.pid = 14;
    s0.state = NOVA_SVC_STATE_RUNNING;
    s0.restart_policy = NOVA_SVC_POLICY_ALWAYS;
    s0.restart_count = 0;
    s0.trusted = 0;

    let s1 = &mut status.services[1];
    let n1 = b"shell";
    s1.name[..n1.len()].copy_from_slice(n1);
    let p1 = b"SHELL.ELF";
    s1.exec_path[..p1.len()].copy_from_slice(p1);
    s1.pid = 18;
    s1.state = NOVA_SVC_STATE_RUNNING;
    s1.restart_policy = NOVA_SVC_POLICY_ALWAYS;
    s1.restart_count = 1;
    s1.trusted = 1;

    // 1. Publish from supervisor PID 1
    let pub_res = rust_service_status_publish(&status, 1);
    if pub_res != 0 {
        return 1;
    }

    // 2. Query snapshot
    let mut queried = ServiceStatusTable::new();
    let q_res = rust_service_status_query(&mut queried);
    if q_res != 0 || queried.count != 2 || queried.services[0].pid != 14 || queried.services[1].pid != 18 {
        return 2;
    }

    // 3. Dispatch control action (restart "shell")
    let ctl_res = rust_service_status_control(NOVA_SVC_ACTION_RESTART, n1.as_ptr(), n1.len() as u32);
    if ctl_res != 0 {
        return 3;
    }

    // 4. Retrieve pending action
    let mut action = 0u32;
    let mut act_name = [0u8; NOVA_SVC_MAX_NAME];
    let get_res = rust_service_status_get_action(&mut action, act_name.as_mut_ptr(), act_name.len() as u32);
    if get_res != 0 || action != NOVA_SVC_ACTION_RESTART || &act_name[..n1.len()] != n1 {
        return 4;
    }

    // 5. Verify pending action was cleared
    let mut action2 = 99u32;
    let _ = rust_service_status_get_action(&mut action2, act_name.as_mut_ptr(), act_name.len() as u32);
    if action2 != NOVA_SVC_ACTION_NONE {
        return 5;
    }

    // 6. Test unauthorized caller (PID 42 without root)
    let denied_res = rust_service_status_publish(&status, 42);
    if denied_res == 0 {
        return 6;
    }

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_status_table_initialization() {
        let table = ServiceStatusTable::new();
        assert_eq!(table.count, 0);
        assert_eq!(table.supervisor_pid, 1);
        assert_eq!(table.pending_action, NOVA_SVC_ACTION_NONE);
    }

    #[test]
    fn test_service_status_roundtrip_and_selftest() {
        assert_eq!(rust_service_status_selftest(), 0);
    }
}
