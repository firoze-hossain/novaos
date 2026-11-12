//! kernel/src/syscall/mod.rs
//!
//! NovaOS 2.0: Subsystem-Specific System Call Handlers

pub mod drm;

pub use drm::{
    dispatch_drm_syscall, run_dmabuf_selftest, SYS_DMABUF_ALLOC, SYS_DMABUF_ATTACH,
    SYS_DMABUF_EXPORT, SYS_DMABUF_FREE, SYS_DMABUF_IMPORT, SYS_DMABUF_INFO, SYS_DMABUF_MAP,
    SYS_DMABUF_SYNC, SYS_DMABUF_UNMAP,
};
