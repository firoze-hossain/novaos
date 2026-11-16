//! kernel/src/syscall/unshare.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): SYS_UNSHARE System Call Handler
//!
//! Disassociates parts of the calling process's execution context and creates new
//! private namespaces for the specified CLONE_NEW* flags.

#![allow(dead_code)]

#[cfg(not(test))]
use crate::security::namespace::{
    global_unshare, CLONE_ALL_NAMESPACES, CLONE_NEWCGROUP, EINVAL,
};

#[cfg(test)]
use crate::security_ns::{
    global_unshare, CLONE_ALL_NAMESPACES, CLONE_NEWCGROUP, EINVAL,
};

pub const SYS_UNSHARE: u64 = 94;
pub const LINUX_SYS_UNSHARE: u64 = 272;

/// Handler for the unshare(flags) system call.
///
/// Unshares execution contexts:
///   - `CLONE_NEWNS`: New isolated mount table and chroot jail.
///   - `CLONE_NEWUTS`: New isolated hostname and domainname.
///   - `CLONE_NEWIPC`: New isolated shared memory and message queues.
///   - `CLONE_NEWUSER`: New isolated user namespace with mapped root.
///   - `CLONE_NEWPID`: New child PID namespace (for subsequent forks).
///   - `CLONE_NEWNET`: New isolated network stack with loopback.
pub fn sys_unshare(caller_pid: u32, flags: u32) -> isize {
    // Check that only supported flags are passed
    let supported = CLONE_ALL_NAMESPACES | CLONE_NEWCGROUP;
    if (flags & !supported) != 0 {
        return EINVAL;
    }

    if flags == 0 {
        return 0; // No-op
    }

    global_unshare(caller_pid, flags)
}
