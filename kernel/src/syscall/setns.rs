//! kernel/src/syscall/setns.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): SYS_SETNS System Call Handler
//!
//! Reassociates the calling process with an existing namespace of the specified type.

#![allow(dead_code)]

#[cfg(not(test))]
use crate::security::namespace::{
    global_setns, CLONE_NEWCGROUP, CLONE_NEWIPC, CLONE_NEWNET,
    CLONE_NEWNS, CLONE_NEWPID, CLONE_NEWUSER, CLONE_NEWUTS, EINVAL,
};

#[cfg(test)]
use crate::security_ns::{
    global_setns, CLONE_NEWCGROUP, CLONE_NEWIPC, CLONE_NEWNET,
    CLONE_NEWNS, CLONE_NEWPID, CLONE_NEWUSER, CLONE_NEWUTS, EINVAL,
};

pub const SYS_SETNS: u64 = 95;
pub const LINUX_SYS_SETNS: u64 = 308;

/// Handler for the setns(fd_or_id, nstype) system call.
///
/// Reassociates the caller with an existing namespace:
///   - `target_ns_id`: ID of target namespace (or file descriptor).
///   - `nstype`: 0 to allow any namespace type, or explicit `CLONE_NEW*` flag.
pub fn sys_setns(caller_pid: u32, target_ns_id: u32, nstype: u32) -> isize {
    let valid_types = [
        0,
        CLONE_NEWNS,
        CLONE_NEWUTS,
        CLONE_NEWIPC,
        CLONE_NEWUSER,
        CLONE_NEWPID,
        CLONE_NEWNET,
        CLONE_NEWCGROUP,
    ];

    let mut is_valid = false;
    for &t in valid_types.iter() {
        if nstype == t {
            is_valid = true;
            break;
        }
    }

    if !is_valid {
        return EINVAL;
    }

    global_setns(caller_pid, target_ns_id, nstype)
}
