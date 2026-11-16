//! kernel/src/syscall/mod.rs
//!
//! NovaOS 2.0: Subsystem-Specific System Call Handlers

pub mod drm;
pub mod unshare;
pub mod setns;

pub use drm::{
    dispatch_drm_syscall, run_dmabuf_selftest, SYS_DMABUF_ALLOC, SYS_DMABUF_ATTACH,
    SYS_DMABUF_EXPORT, SYS_DMABUF_FREE, SYS_DMABUF_IMPORT, SYS_DMABUF_INFO, SYS_DMABUF_MAP,
    SYS_DMABUF_SYNC, SYS_DMABUF_UNMAP,
};

pub use unshare::{sys_unshare, LINUX_SYS_UNSHARE, SYS_UNSHARE};
pub use setns::{sys_setns, LINUX_SYS_SETNS, SYS_SETNS};

#[cfg(not(test))]
use crate::security::namespace::{
    global_clone_process, global_get_context,
};

pub const SYS_NS_CLONE: u64 = 96;
pub const SYS_NS_INFO: u64 = 97;
pub const LINUX_SYS_CLONE: u64 = 56;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct SysNamespaceInfo {
    pub host_pid: u32,
    pub pid_ns: u32,
    pub mount_ns: u32,
    pub net_ns: u32,
    pub ipc_ns: u32,
    pub uts_ns: u32,
    pub user_ns: u32,
}

/// Generic C-ABI namespace syscall dispatcher
pub fn dispatch_namespace_syscall(
    caller_pid: u32,
    syscall_nr: u64,
    arg1: u64,
    arg2: u64,
    _arg3: u64,
) -> isize {
    match syscall_nr {
        SYS_UNSHARE | LINUX_SYS_UNSHARE => sys_unshare(caller_pid, arg1 as u32),
        SYS_SETNS | LINUX_SYS_SETNS => sys_setns(caller_pid, arg1 as u32, arg2 as u32),
        SYS_NS_CLONE | LINUX_SYS_CLONE => {
            #[cfg(not(test))]
            {
                let child_pid = arg1 as u32;
                let clone_flags = arg2 as u32;
                match global_clone_process(caller_pid, child_pid, clone_flags) {
                    Ok(local_pid) => local_pid as isize,
                    Err(e) => e,
                }
            }
            #[cfg(test)]
            {
                0
            }
        }
        SYS_NS_INFO => {
            #[cfg(not(test))]
            {
                let target_pid = arg1 as u32;
                let out_ptr = arg2 as *mut SysNamespaceInfo;
                if out_ptr.is_null() {
                    return -14; // EFAULT
                }
                match global_get_context(target_pid) {
                    Some(ctx) => {
                        unsafe {
                            (*out_ptr).host_pid = ctx.host_pid;
                            (*out_ptr).pid_ns = ctx.pid_ns;
                            (*out_ptr).mount_ns = ctx.mount_ns;
                            (*out_ptr).net_ns = ctx.net_ns;
                            (*out_ptr).ipc_ns = ctx.ipc_ns;
                            (*out_ptr).uts_ns = ctx.uts_ns;
                            (*out_ptr).user_ns = ctx.user_ns;
                        }
                        0
                    }
                    None => -3, // ESRCH
                }
            }
            #[cfg(test)]
            {
                0
            }
        }
        _ => -38, // ENOSYS
    }
}
