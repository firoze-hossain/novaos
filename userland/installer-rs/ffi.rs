//! userland/installer-rs/ffi.rs - FFI bindings and mock environment for NovaOS Installer.
//! Exposes binary ABI matching `novasys.h`, `nova_proc_abi.h`, and framebuffer syscalls.

#![allow(dead_code, static_mut_refs)]

pub const SYS_DISK_INSTALL: u32 = 81;
pub const SYS_DEV_INFO: u32 = 83;
pub const SYS_PROC_INFO: u32 = 84;

pub const DISK_INSTALL_OP_QUERY: i32 = 1;
pub const DISK_INSTALL_OP_DRY_RUN: i32 = 2;
pub const DISK_INSTALL_OP_FORMAT: i32 = 3;
pub const DISK_INSTALL_OP_SELFTEST: i32 = 4;

pub const DISK_INSTALL_OK: i32 = 0;
pub const DISK_INSTALL_ERR_INVALID_PARAM: i32 = -1;
pub const DISK_INSTALL_ERR_DISK_TOO_SMALL: i32 = -2;
pub const DISK_INSTALL_ERR_DEVICE_NOT_FOUND: i32 = -3;
pub const DISK_INSTALL_ERR_IO_WRITE: i32 = -4;
pub const DISK_INSTALL_ERR_IO_READ: i32 = -5;
pub const DISK_INSTALL_ERR_PERMISSION: i32 = -6;
pub const DISK_INSTALL_ERR_VERIFY_FAIL: i32 = -7;

pub const BLOCKDEV_ATA: i32 = 0;
pub const BLOCKDEV_VIRTIO_BLK: i32 = 1;
pub const BLOCKDEV_NVME: i32 = 2;

pub const NOVA_PROC_OP_GET_SYSINFO: u32 = 1;

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiskLayoutConfig {
    pub target_device: i32,    /* 0 = ATA, 1 = VirtIO-Blk, 2 = NVMe */
    pub total_sectors: u32,    /* 0 = auto/default (102MiB = 208,896 sectors) */
    pub part1_mb: u32,         /* FAT32 size in MB (default 64) */
    pub part2_mb: u32,         /* ext2 size in MB (default 32) */
    pub part3_mb: u32,         /* Journal size in MB (default 4) */
    pub part4_mb: u32,         /* Crashdump size in MB (default 1) */
    pub dry_run: u32,          /* 1 = validate only, 0 = live write */
    pub format_fat32: u32,     /* 1 = format FAT32, 0 = partition table only */
    pub format_ext2: u32,      /* 1 = format ext2, 0 = skip */
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiskInstallSummary {
    pub status: i32,           /* 0 on success, < 0 on error */
    pub target_device: i32,
    pub total_sectors: u32,
    pub part1_start_lba: u32,
    pub part1_sectors: u32,
    pub part2_start_lba: u32,
    pub part2_sectors: u32,
    pub part3_start_lba: u32,
    pub part3_sectors: u32,
    pub part4_start_lba: u32,
    pub part4_sectors: u32,
    pub fat_size_sectors: u32,
    pub total_clusters: u32,
    pub sectors_written: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NovaSysActivity {
    pub uptime_ticks: u32,
    pub cpu_count: u32,
    pub cpu_usage_pct: u32,
    pub total_frames: u32,
    pub used_frames: u32,
    pub free_frames: u32,
    pub total_processes: u32,
    pub running_processes: u32,
    pub user_processes: u32,
    pub kernel_processes: u32,
    pub context_switches: u32,
}

impl NovaSysActivity {
    pub const fn empty() -> Self {
        NovaSysActivity {
            uptime_ticks: 0,
            cpu_count: 1,
            cpu_usage_pct: 0,
            total_frames: 0,
            used_frames: 0,
            free_frames: 0,
            total_processes: 0,
            running_processes: 0,
            user_processes: 0,
            kernel_processes: 0,
            context_switches: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MouseState {
    pub dx: i32,
    pub dy: i32,
    pub left_button: bool,
    pub right_button: bool,
    pub middle_button: bool,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct NovaFbInfo {
    pub struct_size: u32,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub backend: u32,
    pub backend_mask: u32,
    pub caps: u32,
    pub max_surface_dim: u32,
    pub max_surfaces: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct NovaFbCreate {
    pub width: u32,
    pub height: u32,
    pub flags: u32,
    pub handle: u32,
    pub pixels: u32,
    pub stride: u32,
    pub size: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct NovaFbPresent {
    pub handle: u32,
    pub src_x: i32,
    pub src_y: i32,
    pub width: i32,
    pub height: i32,
    pub dst_x: i32,
    pub dst_y: i32,
    pub flags: u32,
}

pub const NOVA_FB_BACKEND_ANY: u32 = 0;

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
pub struct RtcTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

// ===========================================================================
// Bare-Metal Syscalls (not(test))
// ===========================================================================

#[cfg(not(test))]
extern "C" {
    pub fn sys_disk_install(op: i32, arg1: *const u8, arg2: *mut u8) -> i32;
    pub fn sys_write(s: *const u8) -> i32;
    pub fn sys_exit(code: i32) -> !;
    pub fn sys_yield();
    pub fn sys_open(filename: *const u8) -> i32;
    pub fn sys_read(handle: i32, buf: *mut u8, max_len: i32) -> i32;
    pub fn sys_close(handle: i32);
    pub fn sys_write_file(filename: *const u8, data: *const u8, size: u32) -> i32;

    pub fn sys_fb_info(out: *mut NovaFbInfo) -> i32;
    pub fn sys_fb_acquire(backend: u32) -> i32;
    pub fn sys_fb_release() -> i32;
    pub fn sys_fb_create(req: *mut NovaFbCreate) -> i32;
    pub fn sys_fb_destroy(handle: u32) -> i32;
    pub fn sys_fb_present(req: *const NovaFbPresent) -> i32;

    pub fn sys_read_key() -> i32;
    pub fn sys_mouse_read(out: *mut MouseState) -> i32;
    pub fn sys_proc_info(op: u32, arg0: u32, buf: *mut u8, len: u32) -> i32;
    pub fn sys_rtc_read(out: *mut RtcTime) -> i32;
    pub fn sys_shutdown() -> i32;
}

// ===========================================================================
// Host Unit Testing Mocks (cfg(test))
// ===========================================================================

#[cfg(test)]
pub static mut MOCK_SYSINFO: NovaSysActivity = NovaSysActivity {
    uptime_ticks: 12000,
    cpu_count: 2,
    cpu_usage_pct: 15,
    total_frames: 65536,  // 256 MB RAM
    used_frames: 8192,    // 32 MB used
    free_frames: 57344,   // 224 MB free
    total_processes: 6,
    running_processes: 2,
    user_processes: 3,
    kernel_processes: 3,
    context_switches: 25000,
};

#[cfg(test)]
pub static mut MOCK_WRITTEN_FILE_LEN: usize = 0;
#[cfg(test)]
pub static mut MOCK_WRITTEN_FILE_BUF: [u8; 1024] = [0; 1024];

#[cfg(test)]
pub unsafe fn reset_mock_environment() {
    MOCK_WRITTEN_FILE_LEN = 0;
    MOCK_WRITTEN_FILE_BUF = [0; 1024];
}

#[cfg(test)]
pub unsafe fn sys_disk_install(op: i32, arg1: *const u8, arg2: *mut u8) -> i32 {
    match op {
        DISK_INSTALL_OP_QUERY => {
            let target_dev = arg1 as usize as i32;
            if arg2.is_null() {
                return DISK_INSTALL_ERR_INVALID_PARAM;
            }
            let summary = &mut *(arg2 as *mut DiskInstallSummary);
            summary.status = DISK_INSTALL_OK;
            summary.target_device = target_dev;
            summary.total_sectors = 208896; // 102 MiB standard test disk
            summary.part1_start_lba = 2048;
            summary.part1_sectors = 64 * 2048;
            summary.part2_start_lba = 2048 + 64 * 2048;
            summary.part2_sectors = 32 * 2048;
            summary.part3_start_lba = 2048 + 96 * 2048;
            summary.part3_sectors = 4 * 2048;
            summary.part4_start_lba = 2048 + 100 * 2048;
            summary.part4_sectors = 1 * 2048;
            summary.fat_size_sectors = 129;
            summary.total_clusters = 16348;
            summary.sectors_written = 0;
            0
        }
        DISK_INSTALL_OP_DRY_RUN => {
            if arg1.is_null() || arg2.is_null() {
                return DISK_INSTALL_ERR_INVALID_PARAM;
            }
            let cfg = &*(arg1 as *const DiskLayoutConfig);
            let summary = &mut *(arg2 as *mut DiskInstallSummary);

            let p1_mb = if cfg.part1_mb == 0 { 64 } else { cfg.part1_mb };
            let p2_mb = if cfg.part2_mb == 0 { 32 } else { cfg.part2_mb };
            let p3_mb = if cfg.part3_mb == 0 { 4 } else { cfg.part3_mb };
            let p4_mb = if cfg.part4_mb == 0 { 1 } else { cfg.part4_mb };

            let p1_sectors = p1_mb * 2048;
            let p2_sectors = p2_mb * 2048;
            let p3_sectors = p3_mb * 2048;
            let p4_sectors = p4_mb * 2048;

            let p1_start = 2048u32;
            let p2_start = p1_start + p1_sectors;
            let p3_start = p2_start + p2_sectors;
            let p4_start = p3_start + p3_sectors;
            let total = p4_start + p4_sectors + 2048;

            summary.status = DISK_INSTALL_OK;
            summary.target_device = cfg.target_device;
            summary.total_sectors = if cfg.total_sectors == 0 { total } else { cfg.total_sectors };
            summary.part1_start_lba = p1_start;
            summary.part1_sectors = p1_sectors;
            summary.part2_start_lba = p2_start;
            summary.part2_sectors = p2_sectors;
            summary.part3_start_lba = p3_start;
            summary.part3_sectors = p3_sectors;
            summary.part4_start_lba = p4_start;
            summary.part4_sectors = p4_sectors;
            summary.fat_size_sectors = ((p1_sectors / 8 * 4 + 511) / 512).max(64);
            summary.total_clusters = (p1_sectors - (32 + 2 * summary.fat_size_sectors)) / 8;
            summary.sectors_written = 0;
            0
        }
        DISK_INSTALL_OP_FORMAT => {
            if arg1.is_null() || arg2.is_null() {
                return DISK_INSTALL_ERR_INVALID_PARAM;
            }
            let _ = sys_disk_install(DISK_INSTALL_OP_DRY_RUN, arg1, arg2);
            let summary = &mut *(arg2 as *mut DiskInstallSummary);
            summary.sectors_written = 1 + summary.part1_sectors + summary.part2_sectors + summary.part3_sectors + summary.part4_sectors;
            0
        }
        DISK_INSTALL_OP_SELFTEST => 0,
        _ => DISK_INSTALL_ERR_INVALID_PARAM,
    }
}

#[cfg(test)]
pub unsafe fn sys_write(_s: *const u8) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_exit(_code: i32) -> ! {
    panic!("sys_exit called in test");
}

#[cfg(test)]
pub unsafe fn sys_yield() {}

#[cfg(test)]
pub unsafe fn sys_open(_filename: *const u8) -> i32 {
    -1
}

#[cfg(test)]
pub unsafe fn sys_read(_handle: i32, _buf: *mut u8, _max_len: i32) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_close(_handle: i32) {}

#[cfg(test)]
pub unsafe fn sys_write_file(_filename: *const u8, data: *const u8, size: u32) -> i32 {
    let copy_len = (size as usize).min(MOCK_WRITTEN_FILE_BUF.len());
    let src = core::slice::from_raw_parts(data, copy_len);
    MOCK_WRITTEN_FILE_BUF[..copy_len].copy_from_slice(src);
    MOCK_WRITTEN_FILE_LEN = copy_len;
    size as i32
}

#[cfg(test)]
pub unsafe fn sys_fb_info(out: *mut NovaFbInfo) -> i32 {
    if !out.is_null() {
        (*out).width = 1024;
        (*out).height = 768;
    }
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_acquire(_backend: u32) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_release() -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_create(req: *mut NovaFbCreate) -> i32 {
    if !req.is_null() {
        (*req).handle = 1;
    }
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_destroy(_handle: u32) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_present(_req: *const NovaFbPresent) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_read_key() -> i32 {
    -1
}

#[cfg(test)]
pub unsafe fn sys_mouse_read(_out: *mut MouseState) -> i32 {
    -1
}

#[cfg(test)]
pub unsafe fn sys_proc_info(op: u32, _arg0: u32, buf: *mut u8, _len: u32) -> i32 {
    if op == NOVA_PROC_OP_GET_SYSINFO && !buf.is_null() {
        *(buf as *mut NovaSysActivity) = MOCK_SYSINFO;
        return 0;
    }
    -22
}

#[cfg(test)]
pub unsafe fn sys_rtc_read(out: *mut RtcTime) -> i32 {
    if !out.is_null() {
        *out = RtcTime {
            year: 2026,
            month: 10,
            day: 28,
            hour: 12,
            minute: 0,
            second: 0,
        };
    }
    0
}

#[cfg(test)]
pub unsafe fn sys_shutdown() -> i32 {
    0
}
