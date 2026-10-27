//! userland/actmon-rs/ffi.rs - FFI bindings and mock environment for Activity Monitor.
//!
//! Exposes binary ABI matching `nova_proc_abi.h` for `SYS_PROC_INFO` (84),
//! framebuffer presentation syscalls, and host mock utilities.

#![allow(dead_code, static_mut_refs)]

pub const NOVA_SYS_PROC_INFO: u32 = 84;

pub const NOVA_PROC_OP_GET_SYSINFO: u32 = 1;
pub const NOVA_PROC_OP_GET_TABLE: u32 = 2;
pub const NOVA_PROC_OP_GET_ENTRY: u32 = 3;

pub const NOVA_PROC_MAX_ENTRIES: usize = 64;
pub const NOVA_PROC_NAME_MAX: usize = 32;

pub const NOVA_PROC_STATE_UNUSED: u32 = 0;
pub const NOVA_PROC_STATE_ALLOCATING: u32 = 1;
pub const NOVA_PROC_STATE_READY: u32 = 2;
pub const NOVA_PROC_STATE_RUNNING: u32 = 3;
pub const NOVA_PROC_STATE_TERMINATED: u32 = 4;

/// System-Wide Activity & Performance Metrics matching `nova_proc_abi.h`.
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

/// Single Process Activity Record matching `nova_proc_abi.h`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NovaProcEntry {
    pub pid: u32,
    pub name: [u8; NOVA_PROC_NAME_MAX],
    pub state: u32,
    pub is_user: u32,
    pub uid: u32,
    pub gid: u32,
    pub cpu_ticks: u32,
    pub cpu_percent: u32,
    pub mem_pages: u32,
    pub mem_bytes: u32,
    pub throttled: u32,
    pub off_cpu: u32,
}

impl NovaProcEntry {
    pub const fn empty() -> Self {
        NovaProcEntry {
            pid: 0,
            name: [0; NOVA_PROC_NAME_MAX],
            state: NOVA_PROC_STATE_UNUSED,
            is_user: 0,
            uid: 0,
            gid: 0,
            cpu_ticks: 0,
            cpu_percent: 0,
            mem_pages: 0,
            mem_bytes: 0,
            throttled: 0,
            off_cpu: 0,
        }
    }

    pub fn name_str(&self) -> &str {
        let mut len = 0;
        while len < NOVA_PROC_NAME_MAX && self.name[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.name[..len]).unwrap_or("unknown")
    }
}

/// Process Table Snapshot matching `nova_proc_abi.h`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NovaProcTable {
    pub count: u32,
    pub entries: [NovaProcEntry; NOVA_PROC_MAX_ENTRIES],
}

impl NovaProcTable {
    pub const fn empty() -> Self {
        NovaProcTable {
            count: 0,
            entries: [NovaProcEntry::empty(); NOVA_PROC_MAX_ENTRIES],
        }
    }
}

// ===========================================================================
// Graphics & Input ABI
// ===========================================================================

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

// ===========================================================================
// Bare-Metal Syscalls (not(test))
// ===========================================================================

#[cfg(not(test))]
extern "C" {
    pub fn sys_write(s: *const u8) -> i32;
    pub fn sys_yield();
    pub fn sys_read_key() -> i32;
    pub fn sys_mouse_read(out: *mut MouseState) -> i32;

    pub fn sys_fb_info(out: *mut NovaFbInfo) -> i32;
    pub fn sys_fb_acquire(backend: u32) -> i32;
    pub fn sys_fb_release() -> i32;
    pub fn sys_fb_create(req: *mut NovaFbCreate) -> i32;
    pub fn sys_fb_destroy(handle: u32) -> i32;
    pub fn sys_fb_present(req: *const NovaFbPresent) -> i32;

    pub fn sys_proc_info(op: u32, arg0: u32, buf: *mut u8, len: u32) -> i32;
    pub fn sys_proc_get_sysinfo(out: *mut NovaSysActivity) -> i32;
    pub fn sys_proc_get_table(out: *mut NovaProcTable) -> i32;
    pub fn sys_proc_get_entry(pid: u32, out: *mut NovaProcEntry) -> i32;
}

// ===========================================================================
// Host Unit Testing Mocks (cfg(test))
// ===========================================================================

#[cfg(test)]
pub static mut MOCK_SYSINFO: NovaSysActivity = NovaSysActivity::empty();
#[cfg(test)]
pub static mut MOCK_PROC_TABLE: NovaProcTable = NovaProcTable::empty();

#[cfg(test)]
pub unsafe fn reset_mock_environment() {
    MOCK_SYSINFO = NovaSysActivity {
        uptime_ticks: 36000, // 360 seconds (6 minutes)
        cpu_count: 2,
        cpu_usage_pct: 28,   // 28% CPU load
        total_frames: 131072, // 512 MB
        used_frames: 20480,   // 80 MB
        free_frames: 110592,  // 432 MB
        total_processes: 8,
        running_processes: 3,
        user_processes: 5,
        kernel_processes: 3,
        context_switches: 72000,
    };

    let mut t = NovaProcTable::empty();
    t.count = 8;

    // Helper macro to populate entries
    let mock_data: &[(u32, &[u8], u32, u32, u32, u32, u32, u32, u32, u32, u32, u32)] = &[
        // pid, name, state, is_user, uid, gid, cpu_ticks, cpu_pct, mem_pages, mem_bytes, throttled, off_cpu
        (0, b"idle", NOVA_PROC_STATE_RUNNING, 0, 0, 0, 25920, 72, 4, 16384, 0, 0),
        (1, b"novainit", NOVA_PROC_STATE_READY, 1, 0, 0, 720, 2, 32, 131072, 0, 1),
        (2, b"SHELL.ELF", NOVA_PROC_STATE_READY, 1, 0, 0, 1800, 5, 48, 196608, 0, 1),
        (3, b"WM.ELF", NOVA_PROC_STATE_RUNNING, 1, 0, 0, 4320, 12, 256, 1048576, 0, 0),
        (4, b"FILES.ELF", NOVA_PROC_STATE_READY, 1, 0, 0, 1440, 4, 128, 524288, 0, 1),
        (5, b"SETTINGS.ELF", NOVA_PROC_STATE_READY, 1, 0, 0, 1080, 3, 96, 393216, 0, 1),
        (6, b"klogd", NOVA_PROC_STATE_READY, 0, 0, 0, 360, 1, 16, 65536, 0, 1),
        (7, b"netd", NOVA_PROC_STATE_READY, 0, 0, 0, 360, 1, 24, 98304, 0, 1),
    ];

    for (i, d) in mock_data.iter().enumerate() {
        t.entries[i].pid = d.0;
        let len = d.1.len().min(NOVA_PROC_NAME_MAX - 1);
        t.entries[i].name[..len].copy_from_slice(&d.1[..len]);
        t.entries[i].name[len] = 0;
        t.entries[i].state = d.2;
        t.entries[i].is_user = d.3;
        t.entries[i].uid = d.4;
        t.entries[i].gid = d.5;
        t.entries[i].cpu_ticks = d.6;
        t.entries[i].cpu_percent = d.7;
        t.entries[i].mem_pages = d.8;
        t.entries[i].mem_bytes = d.9;
        t.entries[i].throttled = d.10;
        t.entries[i].off_cpu = d.11;
    }

    MOCK_PROC_TABLE = t;
}

#[cfg(test)]
pub unsafe fn sys_write(_s: *const u8) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_yield() {}

#[cfg(test)]
pub unsafe fn sys_read_key() -> i32 {
    -1
}

#[cfg(test)]
pub unsafe fn sys_mouse_read(_out: *mut MouseState) -> i32 {
    -1
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
pub unsafe fn sys_proc_get_sysinfo(out: *mut NovaSysActivity) -> i32 {
    if out.is_null() {
        return -14;
    }
    *out = MOCK_SYSINFO;
    0
}

#[cfg(test)]
pub unsafe fn sys_proc_get_table(out: *mut NovaProcTable) -> i32 {
    if out.is_null() {
        return -14;
    }
    *out = MOCK_PROC_TABLE;
    0
}

#[cfg(test)]
pub unsafe fn sys_proc_get_entry(pid: u32, out: *mut NovaProcEntry) -> i32 {
    if out.is_null() {
        return -14;
    }
    for i in 0..(MOCK_PROC_TABLE.count as usize) {
        if MOCK_PROC_TABLE.entries[i].pid == pid {
            *out = MOCK_PROC_TABLE.entries[i];
            return 0;
        }
    }
    -3 // -ESRCH
}

#[cfg(test)]
pub unsafe fn sys_proc_info(op: u32, arg0: u32, buf: *mut u8, _len: u32) -> i32 {
    match op {
        NOVA_PROC_OP_GET_SYSINFO => sys_proc_get_sysinfo(buf as *mut NovaSysActivity),
        NOVA_PROC_OP_GET_TABLE => sys_proc_get_table(buf as *mut NovaProcTable),
        NOVA_PROC_OP_GET_ENTRY => sys_proc_get_entry(arg0, buf as *mut NovaProcEntry),
        _ => -22,
    }
}
