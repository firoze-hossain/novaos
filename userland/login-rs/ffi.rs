//! userland/login-rs/ffi.rs - FFI bindings to NovaOS syscalls (Authentication, VFS, graphics, mouse, RTC).

#![allow(dead_code, static_mut_refs)]

/// Matches kernel/drivers/mouse/ps2mouse.h's mouse_state_t.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct MouseState {
    pub dx: i32,
    pub dy: i32,
    pub left_button: bool,
    pub right_button: bool,
    pub middle_button: bool,
}

/// Matches kernel/drivers/rtc/rtc.h's rtc_time_t.
#[repr(C)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtcTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// Matches nova_fb_abi.h's nova_fb_info_t (36 bytes).
#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
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

/// Matches nova_fb_abi.h's nova_fb_create_t (28 bytes).
#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
pub struct NovaFbCreate {
    pub width: u32,
    pub height: u32,
    pub flags: u32,
    pub handle: u32,
    pub pixels: u32,
    pub stride: u32,
    pub size: u32,
}

/// Matches nova_fb_abi.h's nova_fb_present_t (32 bytes).
#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
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

#[cfg(not(test))]
extern "C" {
    pub fn sys_write(s: *const u8) -> i32;
    pub fn sys_yield();
    pub fn sys_read_key() -> i32;
    pub fn sys_mouse_read(out: *mut MouseState) -> i32;

    // Authentication & Identity
    pub fn sys_login(username: *const u8, password: *const u8) -> i32;
    pub fn sys_getuid() -> u32;

    // System Control
    pub fn sys_shutdown() -> i32;

    // Real-Time Clock
    pub fn sys_rtc_read(out: *mut RtcTime) -> i32;

    // VFS syscalls
    pub fn sys_open(filename: *const u8) -> i32;
    pub fn sys_read(handle: i32, buf: *mut u8, max_len: i32) -> i32;
    pub fn sys_close(handle: i32);
    pub fn sys_list_files(buf: *mut u8, buf_size: i32) -> i32;
    pub fn sys_write_file(filename: *const u8, data: *const u8, size: u32) -> i32;
    pub fn sys_delete_file(filename: *const u8) -> i32;

    // Process & Execution
    pub fn sys_exec(path: *const u8, argv: *const *const u8, argc: i32) -> i32;
    pub fn sys_wait(pid: i32) -> i32;
    pub fn sys_exit(code: i32) -> !;

    // Framebuffer graphics API
    pub fn sys_fb_info(out: *mut NovaFbInfo) -> i32;
    pub fn sys_fb_acquire(backend: u32) -> i32;
    pub fn sys_fb_release() -> i32;
    pub fn sys_fb_create(req: *mut NovaFbCreate) -> i32;
    pub fn sys_fb_present(req: *const NovaFbPresent) -> i32;
    pub fn sys_fb_damage(handle: u32, x: i32, y: i32, w: i32, h: i32) -> i32;
}

#[cfg(test)]
pub static mut MOCK_CURRENT_UID: u32 = 0;
#[cfg(test)]
pub static mut MOCK_VFS_FILES: [MockFile; 8] = [
    MockFile::new(), MockFile::new(), MockFile::new(), MockFile::new(),
    MockFile::new(), MockFile::new(), MockFile::new(), MockFile::new(),
];

#[cfg(test)]
#[derive(Clone, Copy)]
pub struct MockFile {
    pub name: [u8; 32],
    pub data: [u8; 1024],
    pub len: usize,
    pub in_use: bool,
}

#[cfg(test)]
impl MockFile {
    pub const fn new() -> Self {
        MockFile {
            name: [0; 32],
            data: [0; 1024],
            len: 0,
            in_use: false,
        }
    }
}

#[cfg(test)]
pub unsafe fn reset_mock_environment() {
    MOCK_CURRENT_UID = 0;
    for f in MOCK_VFS_FILES.iter_mut() {
        *f = MockFile::new();
    }

    // Set up mock USERS.CFG (716 bytes)
    let mut users_data = [0u8; 716];
    users_data[0..4].copy_from_slice(b"NVUS");

    // Account 0: persisted (uid 700, gid 700)
    let off0 = 4;
    users_data[off0] = 9; // in_use
    let u0 = b"persisted\0";
    users_data[off0 + 1..off0 + 1 + u0.len()].copy_from_slice(u0);
    users_data[off0 + 33..off0 + 37].copy_from_slice(&700u32.to_le_bytes());
    users_data[off0 + 37..off0 + 41].copy_from_slice(&700u32.to_le_bytes());

    // Account 1: admin (uid 800, gid 0)
    let off1 = 4 + 89;
    users_data[off1] = 5; // in_use
    let u1 = b"admin\0";
    users_data[off1 + 1..off1 + 1 + u1.len()].copy_from_slice(u1);
    users_data[off1 + 33..off1 + 37].copy_from_slice(&800u32.to_le_bytes());
    users_data[off1 + 37..off1 + 41].copy_from_slice(&0u32.to_le_bytes());

    add_mock_file("USERS.CFG", &users_data);

    // Set up mock SYSTEM.CFG (81 bytes)
    let mut sys_data = [0u8; 81];
    sys_data[0..4].copy_from_slice(b"NVCF");
    let host = b"novaos-test\0";
    sys_data[4..4 + host.len()].copy_from_slice(host);
    let user = b"admin\0";
    sys_data[36..36 + user.len()].copy_from_slice(user);
    let init = b"SHELL.ELF\0";
    sys_data[68..68 + init.len()].copy_from_slice(init);

    add_mock_file("SYSTEM.CFG", &sys_data);
}

#[cfg(test)]
pub unsafe fn add_mock_file(name_str: &str, data_slice: &[u8]) {
    for f in MOCK_VFS_FILES.iter_mut() {
        if !f.in_use {
            f.in_use = true;
            let bytes = name_str.as_bytes();
            let n = bytes.len().min(31);
            f.name[..n].copy_from_slice(&bytes[..n]);
            f.name[n] = 0;
            let d_len = data_slice.len().min(1024);
            f.data[..d_len].copy_from_slice(&data_slice[..d_len]);
            f.len = d_len;
            return;
        }
    }
}

#[cfg(test)]
pub unsafe fn sys_write(_s: *const u8) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_yield() {}
#[cfg(test)]
pub unsafe fn sys_read_key() -> i32 { -1 }
#[cfg(test)]
pub unsafe fn sys_mouse_read(out: *mut MouseState) -> i32 {
    if !out.is_null() {
        *out = MouseState::default();
    }
    0
}

#[cfg(test)]
pub unsafe fn sys_login(username: *const u8, password: *const u8) -> i32 {
    let mut ulen = 0;
    while *username.add(ulen) != 0 && ulen < 64 {
        ulen += 1;
    }
    let u_slice = core::slice::from_raw_parts(username, ulen);

    let mut plen = 0;
    while *password.add(plen) != 0 && plen < 64 {
        plen += 1;
    }
    let p_slice = core::slice::from_raw_parts(password, plen);

    if u_slice == b"persisted" && p_slice == b"persisted-pw" {
        MOCK_CURRENT_UID = 700;
        0
    } else if u_slice == b"admin" && p_slice == b"admin-correct-password" {
        MOCK_CURRENT_UID = 800;
        0
    } else {
        -1
    }
}

#[cfg(test)]
pub unsafe fn sys_getuid() -> u32 {
    MOCK_CURRENT_UID
}

#[cfg(test)]
pub unsafe fn sys_shutdown() -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_rtc_read(out: *mut RtcTime) -> i32 {
    if !out.is_null() {
        *out = RtcTime {
            year: 2026,
            month: 10,
            day: 26,
            hour: 12,
            minute: 34,
            second: 56,
        };
    }
    0
}

#[cfg(test)]
pub unsafe fn sys_open(filename: *const u8) -> i32 {
    let mut len = 0;
    while *filename.add(len) != 0 && len < 64 {
        len += 1;
    }
    let query = core::slice::from_raw_parts(filename, len);
    for (idx, f) in MOCK_VFS_FILES.iter().enumerate() {
        if f.in_use {
            let mut f_len = 0;
            while f.name[f_len] != 0 && f_len < 32 {
                f_len += 1;
            }
            if &f.name[..f_len] == query {
                return idx as i32;
            }
        }
    }
    -1
}

#[cfg(test)]
pub unsafe fn sys_read(handle: i32, buf: *mut u8, max_len: i32) -> i32 {
    if handle < 0 || handle as usize >= MOCK_VFS_FILES.len() {
        return -1;
    }
    let f = &MOCK_VFS_FILES[handle as usize];
    if !f.in_use {
        return -1;
    }
    let to_copy = f.len.min(max_len as usize);
    core::ptr::copy_nonoverlapping(f.data.as_ptr(), buf, to_copy);
    to_copy as i32
}

#[cfg(test)]
pub unsafe fn sys_close(_handle: i32) {}

#[cfg(test)]
pub unsafe fn sys_list_files(_buf: *mut u8, _buf_size: i32) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_write_file(_filename: *const u8, _data: *const u8, _size: u32) -> i32 { 1 }
#[cfg(test)]
pub unsafe fn sys_delete_file(_filename: *const u8) -> i32 { 1 }

#[cfg(test)]
pub unsafe fn sys_exec(_path: *const u8, _argv: *const *const u8, _argc: i32) -> i32 { 102 }
#[cfg(test)]
pub unsafe fn sys_wait(_pid: i32) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_exit(code: i32) -> ! {
    std::process::exit(code);
}

#[cfg(test)]
pub unsafe fn sys_fb_info(out: *mut NovaFbInfo) -> i32 {
    if !out.is_null() {
        (*out).width = 1024;
        (*out).height = 768;
        (*out).backend = 1;
    }
    0
}
#[cfg(test)]
pub unsafe fn sys_fb_acquire(_b: u32) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_release() -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_create(req: *mut NovaFbCreate) -> i32 {
    if !req.is_null() {
        (*req).handle = 1;
    }
    0
}
#[cfg(test)]
pub unsafe fn sys_fb_present(_p: *const NovaFbPresent) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_damage(_handle: u32, _x: i32, _y: i32, _w: i32, _h: i32) -> i32 { 0 }
