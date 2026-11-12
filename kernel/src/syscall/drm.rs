//! kernel/src/syscall/drm.rs
//!
//! NovaOS 2.0 (Month 4 - Graphics & DRM): DRM & DMA-BUF Zero-Copy System Call Dispatcher
//!
//! Exposes kernel-to-userland syscalls enabling applications and the display compositor
//! (`novadwm`) to allocate hardware-backed DMA-BUF graphics buffers, pass buffer capabilities
//! across process boundaries, and map identical physical frames simultaneously into client
//! and compositor address spaces without CPU pixel copies:
//!   - `SYS_DMABUF_ALLOC  (85)`: Allocate contiguous or scatter-gather GPU framebuffer pages
//!   - `SYS_DMABUF_EXPORT (86)`: Export GEM/DMA-BUF handle to capability-scoped shareable FD
//!   - `SYS_DMABUF_IMPORT (87)`: Import shareable DMA-BUF FD into compositor/receiver handle table
//!   - `SYS_DMABUF_MAP    (88)`: Map buffer physical pages into caller page table with guard page
//!   - `SYS_DMABUF_UNMAP  (89)`: Unmap virtual DMA-BUF region and release mapping reference
//!   - `SYS_DMABUF_FREE   (90)`: Close local handle / exported FD (reclaims RAM at refcount == 0)
//!   - `SYS_DMABUF_SYNC   (91)`: Cache coherency memory barrier (DMA_BUF_SYNC_START/END)
//!   - `SYS_DMABUF_INFO   (92)`: Query geometry, stride, physical base, refcount, sg segments
//!   - `SYS_DMABUF_ATTACH (93)`: Bind buffer directly to DRM CRTC Scanout or VirtIO-GPU 3D

#![allow(dead_code, non_camel_case_types)]

#[cfg(test)]
#[path = "../drivers/gpu/dmabuf.rs"]
pub mod dmabuf;

#[cfg(not(test))]
use crate::gpu::dmabuf::{
    DmaBufFormat, DmaBufManager, DmaDeviceType, DMABUF_FLAG_CPU_ACCESS, DMABUF_FLAG_SCANOUT,
    DMABUF_FD_BASE, DMABUF_PERM_READ, DMABUF_PROT_READ, DMABUF_PROT_WC,
    DMABUF_PROT_WRITE, DMABUF_VIRT_BASE, DMA_BUF_SYNC_END, DMA_BUF_SYNC_READ,
    DMA_BUF_SYNC_START, DMA_BUF_SYNC_WRITE, EACCES, EFAULT, EINVAL, GLOBAL_DMABUF,
};

#[cfg(test)]
use dmabuf::{
    DmaBufFormat, DmaBufManager, DmaDeviceType, DMABUF_FLAG_CPU_ACCESS, DMABUF_FLAG_SCANOUT,
    DMABUF_FD_BASE, DMABUF_PERM_READ, DMABUF_PROT_READ, DMABUF_PROT_WC,
    DMABUF_PROT_WRITE, DMABUF_VIRT_BASE, DMA_BUF_SYNC_END, DMA_BUF_SYNC_READ,
    DMA_BUF_SYNC_START, DMA_BUF_SYNC_WRITE, EACCES, EFAULT, EINVAL, GLOBAL_DMABUF,
};

// ============================================================================
// System Call Numbers & Canonical Userland Boundaries
// ============================================================================

pub const SYS_DMABUF_ALLOC: u64  = 85;
pub const SYS_DMABUF_EXPORT: u64 = 86;
pub const SYS_DMABUF_IMPORT: u64 = 87;
pub const SYS_DMABUF_MAP: u64    = 88;
pub const SYS_DMABUF_UNMAP: u64  = 89;
pub const SYS_DMABUF_FREE: u64   = 90;
pub const SYS_DMABUF_SYNC: u64   = 91;
pub const SYS_DMABUF_INFO: u64   = 92;
pub const SYS_DMABUF_ATTACH: u64 = 93;

/// Negative ENOSYS code when syscall ID is unrecognized
pub const ERR_ENOSYS: isize = -38;

/// Maximum canonical 48-bit user-space virtual address (128 TiB boundary)
pub const USER_CANONICAL_MAX: u64 = 0x0000_7FFF_FFFF_FFFF;
pub const KERNEL_HIGHER_HALF_BASE: u64 = 0xFFFF_8000_0000_0000;

// ============================================================================
// Syscall C-ABI Request / Response Payloads
// ============================================================================

/// Arguments for `SYS_DMABUF_ALLOC`
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
#[repr(C)]
pub struct SysDmabufAllocArgs {
    pub width: u32,
    pub height: u32,
    pub format: u32,        // DmaBufFormat FourCC
    pub flags: u32,         // DMABUF_FLAG_*
    pub requested_size: u64,
    // Outputs:
    pub out_handle: u32,
    pub out_obj_id: u32,
    pub out_stride: u32,
    pub out_sg_count: u32,
    pub out_aligned_size: u64,
    pub out_phys_base: u64,
}

/// Arguments for `SYS_DMABUF_EXPORT`
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
#[repr(C)]
pub struct SysDmabufExportArgs {
    pub handle: u32,
    pub target_pid: u32,    // Specific PID (e.g., compositor) or 0 (wildcard)
    pub rights: u32,        // DMABUF_PERM_*
    pub out_fd: i32,
}

/// Arguments for `SYS_DMABUF_IMPORT`
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
#[repr(C)]
pub struct SysDmabufImportArgs {
    pub dmabuf_fd: i32,
    pub out_handle: u32,
    pub out_obj_id: u32,
    pub out_width: u32,
    pub out_height: u32,
    pub out_stride: u32,
    pub out_format: u32,
    pub out_size: u64,
}

/// Arguments for `SYS_DMABUF_MAP`
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
#[repr(C)]
pub struct SysDmabufMapArgs {
    pub handle: u32,
    pub prot: u32,          // DMABUF_PROT_READ | WRITE | WC
    pub out_virt_addr: u64,
    pub out_mapped_size: u64,
    pub out_guard_addr: u64,
    pub out_map_count: u32,
    pub _pad: u32,
}

/// Arguments for `SYS_DMABUF_INFO`
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
#[repr(C)]
pub struct SysDmabufInfoArgs {
    pub handle: u32,
    pub out_obj_id: u32,
    pub out_owner_pid: u32,
    pub out_width: u32,
    pub out_height: u32,
    pub out_stride: u32,
    pub out_format: u32,
    pub out_refcount: u32,
    pub out_map_count: u32,
    pub out_sg_count: u32,
    pub out_is_contiguous: u32,
    pub _pad: u32,
    pub out_size: u64,
    pub out_phys_base: u64,
}

/// Arguments for `SYS_DMABUF_ATTACH`
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
#[repr(C)]
pub struct SysDmabufAttachArgs {
    pub handle: u32,
    pub dev_type: u32,      // DmaDeviceType (1=DrmScanout, 2=VirtioGpu3D, 3=VideoDecoder)
    pub device_handle: u32, // CRTC/FB ID or Virgl resource ID
    pub detach: u32,        // 0 = attach, 1 = detach
}

// ============================================================================
// Userland Pointer Boundary Validation
// ============================================================================

/// Validates that a user-provided pointer sits strictly within canonical lower-half
/// user space, is non-null, properly aligned, and does not wrap around 64-bit bounds.
pub fn validate_user_ptr(ptr: u64, len: usize, align: usize) -> Result<(), isize> {
    if len == 0 {
        return Ok(());
    }
    if ptr == 0 {
        return Err(EFAULT as isize);
    }
    let len_u64 = len as u64;
    let end = match ptr.checked_add(len_u64 - 1) {
        Some(e) => e,
        None => return Err(EFAULT as isize),
    };
    if ptr > USER_CANONICAL_MAX || end > USER_CANONICAL_MAX {
        return Err(EFAULT as isize);
    }
    if ptr >= KERNEL_HIGHER_HALF_BASE || end >= KERNEL_HIGHER_HALF_BASE {
        return Err(EFAULT as isize);
    }
    if align > 1 && (ptr as usize) % align != 0 {
        return Err(EINVAL as isize);
    }
    Ok(())
}

// ============================================================================
// Individual Syscall Handlers Operating on DmaBufManager
// ============================================================================

pub fn sys_dmabuf_alloc_on(mgr: &mut DmaBufManager, caller_pid: u32, args_ptr: u64) -> isize {
    if let Err(e) = validate_user_ptr(
        args_ptr,
        core::mem::size_of::<SysDmabufAllocArgs>(),
        core::mem::align_of::<SysDmabufAllocArgs>(),
    ) {
        return e;
    }

    let args = unsafe { &mut *(args_ptr as *mut SysDmabufAllocArgs) };
    let fmt = match DmaBufFormat::from_u32(args.format) {
        Some(f) => f,
        None => return EINVAL as isize,
    };

    match mgr.alloc(
        caller_pid,
        args.width,
        args.height,
        fmt,
        args.requested_size,
        args.flags,
    ) {
        Ok(res) => {
            args.out_handle = res.handle;
            args.out_obj_id = res.obj_id;
            args.out_stride = res.stride;
            args.out_sg_count = res.sg_count;
            args.out_aligned_size = res.aligned_size;
            args.out_phys_base = res.phys_base;
            0
        }
        Err(e) => e as isize,
    }
}

pub fn sys_dmabuf_export_on(mgr: &mut DmaBufManager, caller_pid: u32, args_ptr: u64) -> isize {
    if let Err(e) = validate_user_ptr(
        args_ptr,
        core::mem::size_of::<SysDmabufExportArgs>(),
        core::mem::align_of::<SysDmabufExportArgs>(),
    ) {
        return e;
    }

    let args = unsafe { &mut *(args_ptr as *mut SysDmabufExportArgs) };
    match mgr.export(caller_pid, args.handle, args.target_pid, args.rights) {
        Ok(fd) => {
            args.out_fd = fd;
            fd as isize
        }
        Err(e) => e as isize,
    }
}

pub fn sys_dmabuf_import_on(mgr: &mut DmaBufManager, caller_pid: u32, args_ptr: u64) -> isize {
    if let Err(e) = validate_user_ptr(
        args_ptr,
        core::mem::size_of::<SysDmabufImportArgs>(),
        core::mem::align_of::<SysDmabufImportArgs>(),
    ) {
        return e;
    }

    let args = unsafe { &mut *(args_ptr as *mut SysDmabufImportArgs) };
    match mgr.import(caller_pid, args.dmabuf_fd) {
        Ok(res) => {
            args.out_handle = res.handle;
            args.out_obj_id = res.obj_id;
            args.out_width = res.width;
            args.out_height = res.height;
            args.out_stride = res.stride;
            args.out_format = res.format;
            args.out_size = res.size;
            0
        }
        Err(e) => e as isize,
    }
}

pub fn sys_dmabuf_map_on(mgr: &mut DmaBufManager, caller_pid: u32, args_ptr: u64) -> isize {
    if let Err(e) = validate_user_ptr(
        args_ptr,
        core::mem::size_of::<SysDmabufMapArgs>(),
        core::mem::align_of::<SysDmabufMapArgs>(),
    ) {
        return e;
    }

    let args = unsafe { &mut *(args_ptr as *mut SysDmabufMapArgs) };
    match mgr.map(caller_pid, args.handle, args.prot) {
        Ok(res) => {
            args.out_virt_addr = res.virt_addr;
            args.out_mapped_size = res.mapped_size;
            args.out_guard_addr = res.guard_page_addr;
            args.out_map_count = res.map_count;
            0
        }
        Err(e) => e as isize,
    }
}

pub fn sys_dmabuf_unmap_on(mgr: &mut DmaBufManager, caller_pid: u32, virt_addr: u64) -> isize {
    if virt_addr == 0 || virt_addr > USER_CANONICAL_MAX {
        return EFAULT as isize;
    }
    match mgr.unmap(caller_pid, virt_addr) {
        Ok(()) => 0,
        Err(e) => e as isize,
    }
}

pub fn sys_dmabuf_free_on(
    mgr: &mut DmaBufManager,
    caller_pid: u32,
    handle: u32,
    dmabuf_fd: i32,
) -> isize {
    let mut rc = 0isize;
    if handle != 0 {
        if let Err(e) = mgr.free_handle(caller_pid, handle) {
            rc = e as isize;
        }
    }
    if dmabuf_fd > 0 {
        if let Err(e) = mgr.close_fd(caller_pid, dmabuf_fd) {
            if rc == 0 {
                rc = e as isize;
            }
        }
    }
    if handle == 0 && dmabuf_fd <= 0 {
        return EINVAL as isize;
    }
    rc
}

pub fn sys_dmabuf_sync_on(
    mgr: &mut DmaBufManager,
    caller_pid: u32,
    handle: u32,
    flags: u32,
) -> isize {
    match mgr.sync(caller_pid, handle, flags) {
        Ok(()) => 0,
        Err(e) => e as isize,
    }
}

pub fn sys_dmabuf_info_on(mgr: &DmaBufManager, caller_pid: u32, args_ptr: u64) -> isize {
    if let Err(e) = validate_user_ptr(
        args_ptr,
        core::mem::size_of::<SysDmabufInfoArgs>(),
        core::mem::align_of::<SysDmabufInfoArgs>(),
    ) {
        return e;
    }

    let args = unsafe { &mut *(args_ptr as *mut SysDmabufInfoArgs) };
    match mgr.info(caller_pid, args.handle) {
        Ok(info) => {
            args.out_obj_id = info.obj_id;
            args.out_owner_pid = info.owner_pid;
            args.out_width = info.width;
            args.out_height = info.height;
            args.out_stride = info.stride;
            args.out_format = info.format;
            args.out_refcount = info.refcount;
            args.out_map_count = info.map_count;
            args.out_sg_count = info.sg_count;
            args.out_is_contiguous = if info.is_contiguous { 1 } else { 0 };
            args.out_size = info.size;
            args.out_phys_base = info.phys_base;
            0
        }
        Err(e) => e as isize,
    }
}

pub fn sys_dmabuf_attach_on(mgr: &mut DmaBufManager, caller_pid: u32, args_ptr: u64) -> isize {
    if let Err(e) = validate_user_ptr(
        args_ptr,
        core::mem::size_of::<SysDmabufAttachArgs>(),
        core::mem::align_of::<SysDmabufAttachArgs>(),
    ) {
        return e;
    }

    let args = unsafe { &*(args_ptr as *const SysDmabufAttachArgs) };
    let dev_type = match DmaDeviceType::from_u32(args.dev_type) {
        Some(dt) => dt,
        None => return EINVAL as isize,
    };

    let res = if args.detach != 0 {
        mgr.detach_device(caller_pid, args.handle, dev_type, args.device_handle)
    } else {
        mgr.attach_device(caller_pid, args.handle, dev_type, args.device_handle)
    };
    match res {
        Ok(()) => 0,
        Err(e) => e as isize,
    }
}

/// Main DRM/DMA-BUF syscall dispatch entry point operating on a specific manager instance
pub fn dispatch_drm_syscall_on(
    mgr: &mut DmaBufManager,
    caller_pid: u32,
    syscall_nr: u64,
    arg1: u64,
    arg2: u64,
    _arg3: u64,
) -> isize {
    match syscall_nr {
        SYS_DMABUF_ALLOC => sys_dmabuf_alloc_on(mgr, caller_pid, arg1),
        SYS_DMABUF_EXPORT => sys_dmabuf_export_on(mgr, caller_pid, arg1),
        SYS_DMABUF_IMPORT => sys_dmabuf_import_on(mgr, caller_pid, arg1),
        SYS_DMABUF_MAP => sys_dmabuf_map_on(mgr, caller_pid, arg1),
        SYS_DMABUF_UNMAP => sys_dmabuf_unmap_on(mgr, caller_pid, arg1),
        SYS_DMABUF_FREE => sys_dmabuf_free_on(mgr, caller_pid, arg1 as u32, arg2 as i32),
        SYS_DMABUF_SYNC => sys_dmabuf_sync_on(mgr, caller_pid, arg1 as u32, arg2 as u32),
        SYS_DMABUF_INFO => sys_dmabuf_info_on(mgr, caller_pid, arg1),
        SYS_DMABUF_ATTACH => sys_dmabuf_attach_on(mgr, caller_pid, arg1),
        _ => ERR_ENOSYS,
    }
}

/// Top-level kernel syscall dispatcher targeting `GLOBAL_DMABUF`
pub fn dispatch_drm_syscall(
    caller_pid: u32,
    syscall_nr: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
) -> isize {
    let mgr = unsafe { &mut *core::ptr::addr_of_mut!(GLOBAL_DMABUF) };
    if !mgr.initialized {
        mgr.init();
    }
    dispatch_drm_syscall_on(mgr, caller_pid, syscall_nr, arg1, arg2, arg3)
}

// ============================================================================
// Boot Integration & In-Kernel Self-Test
// ============================================================================

/// NovaOS 2.0 (Month 4 - Graphics & DRM): Zero-Copy DMA-BUF Framebuffer Sharing &
/// GPU Memory Allocator Self-Test.
///
/// Validates:
/// 1. Physical GPU/DMA frame pool initialization (128 MiB, 32,768 pages).
/// 2. `SYS_DMABUF_ALLOC` syscall for a 1920x1080 ARGB8888 scanout framebuffer.
/// 3. `SYS_DMABUF_MAP` syscall in client process space with trailing guard page.
/// 4. `SYS_DMABUF_EXPORT` capability-scoped PRIME FD handoff targeting compositor.
/// 5. `SYS_DMABUF_IMPORT` & `SYS_DMABUF_MAP` in compositor address space (`novadwm`).
/// 6. Exact page-by-page physical frame identity across client & compositor (0 CPU copies).
/// 7. `SYS_DMABUF_SYNC` cache coherency barriers and `SYS_DMABUF_ATTACH` DRM/VirtIO hooks.
/// 8. Strict security enforcement (`-EFAULT` on kernel pointer, `-EACCES` on rogue PID).
/// 9. Crash-safe refcounted teardown releasing 100% of allocated physical frames.
pub fn run_dmabuf_selftest() -> i32 {
    let mgr = unsafe { &mut *core::ptr::addr_of_mut!(GLOBAL_DMABUF) };
    mgr.init();
    let baseline_free_pages = mgr.stats().free_pool_pages;

    let client_pid: u32 = 101;
    let compositor_pid: u32 = 2;
    let rogue_pid: u32 = 999;

    // 1. Validate strict kernel-pointer firewalling (-EFAULT)
    if dispatch_drm_syscall(client_pid, SYS_DMABUF_ALLOC, KERNEL_HIGHER_HALF_BASE + 0x1000, 0, 0)
        != EFAULT as isize
    {
        return 1;
    }

    // 2. Client invokes SYS_DMABUF_ALLOC for 1920x1080 ARGB8888 scanout buffer
    let mut alloc_req = SysDmabufAllocArgs {
        width: 1920,
        height: 1080,
        format: DmaBufFormat::Argb8888 as u32,
        flags: DMABUF_FLAG_SCANOUT | DMABUF_FLAG_CPU_ACCESS,
        requested_size: 0,
        ..SysDmabufAllocArgs::default()
    };
    if dispatch_drm_syscall(
        client_pid,
        SYS_DMABUF_ALLOC,
        &mut alloc_req as *mut _ as u64,
        0,
        0,
    ) != 0
    {
        return 2;
    }
    if alloc_req.out_handle == 0 || alloc_req.out_aligned_size != 8_294_400 {
        return 3;
    }

    // 3. Client maps physical pages into its virtual address space (SYS_DMABUF_MAP)
    let mut client_map_req = SysDmabufMapArgs {
        handle: alloc_req.out_handle,
        prot: DMABUF_PROT_READ | DMABUF_PROT_WRITE | DMABUF_PROT_WC,
        ..SysDmabufMapArgs::default()
    };
    if dispatch_drm_syscall(
        client_pid,
        SYS_DMABUF_MAP,
        &mut client_map_req as *mut _ as u64,
        0,
        0,
    ) != 0
    {
        return 4;
    }
    if client_map_req.out_virt_addr < DMABUF_VIRT_BASE {
        return 5;
    }

    // 4. Client exports shareable FD scoped to compositor PID (SYS_DMABUF_EXPORT)
    let mut export_req = SysDmabufExportArgs {
        handle: alloc_req.out_handle,
        target_pid: compositor_pid,
        rights: DMABUF_PERM_READ,
        out_fd: 0,
    };
    let fd_res = dispatch_drm_syscall(
        client_pid,
        SYS_DMABUF_EXPORT,
        &mut export_req as *mut _ as u64,
        0,
        0,
    );
    if fd_res < DMABUF_FD_BASE as isize || export_req.out_fd < DMABUF_FD_BASE {
        return 6;
    }

    // 5. Unauthorized rogue process cannot hijack the exported FD
    let mut rogue_import = SysDmabufImportArgs {
        dmabuf_fd: export_req.out_fd,
        ..SysDmabufImportArgs::default()
    };
    if dispatch_drm_syscall(
        rogue_pid,
        SYS_DMABUF_IMPORT,
        &mut rogue_import as *mut _ as u64,
        0,
        0,
    ) != EACCES as isize
    {
        return 7;
    }

    // 6. Compositor imports FD (SYS_DMABUF_IMPORT) and maps read-only (SYS_DMABUF_MAP)
    let mut comp_import = SysDmabufImportArgs {
        dmabuf_fd: export_req.out_fd,
        ..SysDmabufImportArgs::default()
    };
    if dispatch_drm_syscall(
        compositor_pid,
        SYS_DMABUF_IMPORT,
        &mut comp_import as *mut _ as u64,
        0,
        0,
    ) != 0
    {
        return 8;
    }
    if comp_import.out_width != 1920 || comp_import.out_stride != 7680 {
        return 9;
    }

    let mut comp_map_req = SysDmabufMapArgs {
        handle: comp_import.out_handle,
        prot: DMABUF_PROT_READ,
        ..SysDmabufMapArgs::default()
    };
    if dispatch_drm_syscall(
        compositor_pid,
        SYS_DMABUF_MAP,
        &mut comp_map_req as *mut _ as u64,
        0,
        0,
    ) != 0
    {
        return 10;
    }

    // 7. Verify 100% Zero-Copy Identity (2025 identical 4KB frames mapped in both processes)
    let proof = match mgr.verify_zero_copy_sharing(
        client_pid,
        client_map_req.out_virt_addr,
        compositor_pid,
        comp_map_req.out_virt_addr,
    ) {
        Ok(p) => p,
        Err(_) => return 11,
    };
    if !proof.is_zero_copy || !proof.guard_verified || proof.verified_pages != 2025 {
        return 12;
    }

    // 8. Cache Coherency Barriers (SYS_DMABUF_SYNC) & Hardware Scanout Attachment (SYS_DMABUF_ATTACH)
    if dispatch_drm_syscall(
        client_pid,
        SYS_DMABUF_SYNC,
        alloc_req.out_handle as u64,
        (DMA_BUF_SYNC_END | DMA_BUF_SYNC_WRITE) as u64,
        0,
    ) != 0
    {
        return 13;
    }
    if dispatch_drm_syscall(
        compositor_pid,
        SYS_DMABUF_SYNC,
        comp_import.out_handle as u64,
        (DMA_BUF_SYNC_START | DMA_BUF_SYNC_READ) as u64,
        0,
    ) != 0
    {
        return 14;
    }

    let attach_req = SysDmabufAttachArgs {
        handle: comp_import.out_handle,
        dev_type: DmaDeviceType::DrmScanout as u32,
        device_handle: 1, // CRTC/FB scanout binding
        detach: 0,
    };
    if dispatch_drm_syscall(
        compositor_pid,
        SYS_DMABUF_ATTACH,
        &attach_req as *const _ as u64,
        0,
        0,
    ) != 0
    {
        return 15;
    }

    // 9. Query live metadata (SYS_DMABUF_INFO)
    let mut info_req = SysDmabufInfoArgs {
        handle: comp_import.out_handle,
        ..SysDmabufInfoArgs::default()
    };
    if dispatch_drm_syscall(
        compositor_pid,
        SYS_DMABUF_INFO,
        &mut info_req as *mut _ as u64,
        0,
        0,
    ) != 0
    {
        return 16;
    }
    if info_req.out_map_count != 2 || info_req.out_is_contiguous != 1 || info_req.out_refcount < 5 {
        return 17;
    }

    // 10. Clean teardown: client unmaps & drops handle, compositor detaches, unmaps & frees
    let detach_req = SysDmabufAttachArgs {
        detach: 1,
        ..attach_req
    };
    if dispatch_drm_syscall(
        compositor_pid,
        SYS_DMABUF_ATTACH,
        &detach_req as *const _ as u64,
        0,
        0,
    ) != 0
    {
        return 18;
    }

    if dispatch_drm_syscall(client_pid, SYS_DMABUF_UNMAP, client_map_req.out_virt_addr, 0, 0) != 0 {
        return 19;
    }
    if dispatch_drm_syscall(
        client_pid,
        SYS_DMABUF_FREE,
        alloc_req.out_handle as u64,
        export_req.out_fd as u64,
        0,
    ) != 0
    {
        return 20;
    }
    if dispatch_drm_syscall(compositor_pid, SYS_DMABUF_UNMAP, comp_map_req.out_virt_addr, 0, 0) != 0
    {
        return 21;
    }
    if dispatch_drm_syscall(
        compositor_pid,
        SYS_DMABUF_FREE,
        comp_import.out_handle as u64,
        0,
        0,
    ) != 0
    {
        return 22;
    }

    // Verify zero leaks
    let end_stats = mgr.stats();
    if end_stats.active_objects != 0
        || end_stats.active_mappings != 0
        || end_stats.free_pool_pages != baseline_free_pages
    {
        return 23;
    }

    0
}

// ============================================================================
// Unit Tests (Host Executable via rustc --test)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_syscall_pointer_boundary_validation() {
        let mut mgr = DmaBufManager::new();
        mgr.init();

        // Null pointer rejected with -EFAULT
        assert_eq!(
            dispatch_drm_syscall_on(&mut mgr, 10, SYS_DMABUF_ALLOC, 0, 0, 0),
            EFAULT as isize
        );

        // Kernel higher-half address rejected with -EFAULT
        assert_eq!(
            dispatch_drm_syscall_on(
                &mut mgr,
                10,
                SYS_DMABUF_ALLOC,
                KERNEL_HIGHER_HALF_BASE + 0x2000,
                0,
                0
            ),
            EFAULT as isize
        );

        // Wrapping 64-bit pointer rejected with -EFAULT
        assert_eq!(
            dispatch_drm_syscall_on(&mut mgr, 10, SYS_DMABUF_IMPORT, u64::MAX - 8, 0, 0),
            EFAULT as isize
        );

        // Unknown syscall ID returns -ENOSYS
        assert_eq!(
            dispatch_drm_syscall_on(&mut mgr, 10, 999, 0, 0, 0),
            ERR_ENOSYS
        );
    }

    #[test]
    fn test_syscall_full_compositor_swapchain_cycle() {
        let mut mgr = DmaBufManager::new();
        mgr.init();
        let baseline = mgr.stats().free_pool_pages;

        let app_pid = 200;
        let compositor_pid = 2;

        // Allocate Double-Buffered Front & Back Buffers via SYS_DMABUF_ALLOC
        let mut alloc_front = SysDmabufAllocArgs {
            width: 1280,
            height: 720,
            format: DmaBufFormat::Argb8888 as u32,
            flags: DMABUF_FLAG_SCANOUT | DMABUF_FLAG_CPU_ACCESS,
            ..SysDmabufAllocArgs::default()
        };
        let mut alloc_back = alloc_front;

        assert_eq!(
            dispatch_drm_syscall_on(
                &mut mgr,
                app_pid,
                SYS_DMABUF_ALLOC,
                &mut alloc_front as *mut _ as u64,
                0,
                0
            ),
            0
        );
        assert_eq!(
            dispatch_drm_syscall_on(
                &mut mgr,
                app_pid,
                SYS_DMABUF_ALLOC,
                &mut alloc_back as *mut _ as u64,
                0,
                0
            ),
            0
        );
        assert_ne!(alloc_front.out_phys_base, alloc_back.out_phys_base);

        // App maps both buffers, writes, and exports to compositor
        let mut app_map = SysDmabufMapArgs {
            handle: alloc_back.out_handle,
            prot: DMABUF_PROT_READ | DMABUF_PROT_WRITE,
            ..SysDmabufMapArgs::default()
        };
        assert_eq!(
            dispatch_drm_syscall_on(
                &mut mgr,
                app_pid,
                SYS_DMABUF_MAP,
                &mut app_map as *mut _ as u64,
                0,
                0
            ),
            0
        );

        let mut exp = SysDmabufExportArgs {
            handle: alloc_back.out_handle,
            target_pid: compositor_pid,
            rights: DMABUF_PERM_READ,
            out_fd: 0,
        };
        assert!(
            dispatch_drm_syscall_on(
                &mut mgr,
                app_pid,
                SYS_DMABUF_EXPORT,
                &mut exp as *mut _ as u64,
                0,
                0
            ) >= DMABUF_FD_BASE as isize
        );

        // Compositor imports & maps the back buffer for scanout
        let mut imp = SysDmabufImportArgs {
            dmabuf_fd: exp.out_fd,
            ..SysDmabufImportArgs::default()
        };
        assert_eq!(
            dispatch_drm_syscall_on(
                &mut mgr,
                compositor_pid,
                SYS_DMABUF_IMPORT,
                &mut imp as *mut _ as u64,
                0,
                0
            ),
            0
        );

        let mut comp_map = SysDmabufMapArgs {
            handle: imp.out_handle,
            prot: DMABUF_PROT_READ,
            ..SysDmabufMapArgs::default()
        };
        assert_eq!(
            dispatch_drm_syscall_on(
                &mut mgr,
                compositor_pid,
                SYS_DMABUF_MAP,
                &mut comp_map as *mut _ as u64,
                0,
                0
            ),
            0
        );

        // Verify zero-copy identical physical frames across back buffer mappings
        let proof = mgr
            .verify_zero_copy_sharing(app_pid, app_map.out_virt_addr, compositor_pid, comp_map.out_virt_addr)
            .unwrap();
        assert!(proof.is_zero_copy);
        assert!(proof.guard_verified);

        // Free everything and verify 100% reclamation
        mgr.cleanup_process(app_pid);
        mgr.cleanup_process(compositor_pid);
        assert_eq!(mgr.stats().active_objects, 0);
        assert_eq!(mgr.stats().free_pool_pages, baseline);
    }

    #[test]
    fn test_in_kernel_selftest_passes() {
        assert_eq!(run_dmabuf_selftest(), 0);
    }
}
