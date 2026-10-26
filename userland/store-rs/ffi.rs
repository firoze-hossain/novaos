//! userland/store-rs/ffi.rs - FFI bindings to NovaOS syscalls (VFS, graphics, mouse, pkgrepo).

#![allow(dead_code)]

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

/// Matches nova_pkgrepo_abi.h's nova_pkgrepo_status_t (20 bytes).
#[repr(C)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct NovaPkgrepoStatus {
    pub is_running: u32,
    pub port: u16,
    pub _reserved: u16,
    pub requests_total: u32,
    pub packages_served: u32,
    pub bytes_served: u32,
}

pub const NOVA_PKGREPO_CTL_START: u32 = 1;
pub const NOVA_PKGREPO_CTL_STOP: u32 = 2;
pub const NOVA_PKGREPO_CTL_STATUS: u32 = 3;
pub const NOVA_PKGREPO_DEFAULT_PORT: u16 = 8080;

pub const NOVA_FB_BACKEND_ANY: u32 = 0;

#[cfg(not(test))]
extern "C" {
    pub fn sys_write(s: *const u8) -> i32;
    pub fn sys_yield();
    pub fn sys_read_key() -> i32;
    pub fn sys_mouse_read(out: *mut MouseState) -> i32;

    // VFS syscalls
    pub fn sys_open(filename: *const u8) -> i32;
    pub fn sys_read(handle: i32, buf: *mut u8, max_len: i32) -> i32;
    pub fn sys_close(handle: i32);
    pub fn sys_list_files(buf: *mut u8, buf_size: i32) -> i32;
    pub fn sys_write_file(filename: *const u8, data: *const u8, size: u32) -> i32;
    pub fn sys_delete_file(filename: *const u8) -> i32;

    // Package Repository Syscall
    pub fn sys_pkgrepo_ctl(cmd: u32, arg: *mut u8) -> i32;

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
pub static mut MOCK_VFS_FILES: [MockFile; 8] = [
    MockFile::new(), MockFile::new(), MockFile::new(), MockFile::new(),
    MockFile::new(), MockFile::new(), MockFile::new(), MockFile::new(),
];
#[cfg(test)]
pub static mut MOCK_VFS_FILE_COUNT: usize = 0;
#[cfg(test)]
pub static mut MOCK_PKGREPO_STATUS: NovaPkgrepoStatus = NovaPkgrepoStatus {
    is_running: 1,
    port: NOVA_PKGREPO_DEFAULT_PORT,
    _reserved: 0,
    requests_total: 42,
    packages_served: 3,
    bytes_served: 12480,
};

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
    MOCK_VFS_FILE_COUNT = 0;
    for i in 0..MOCK_VFS_FILES.len() {
        MOCK_VFS_FILES[i] = MockFile::new();
    }
    MOCK_PKGREPO_STATUS = NovaPkgrepoStatus {
        is_running: 1,
        port: NOVA_PKGREPO_DEFAULT_PORT,
        _reserved: 0,
        requests_total: 42,
        packages_served: 3,
        bytes_served: 12480,
    };
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
            MOCK_VFS_FILE_COUNT += 1;
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
pub unsafe fn sys_list_files(buf: *mut u8, buf_size: i32) -> i32 {
    let mut offset = 0;
    for f in MOCK_VFS_FILES.iter() {
        if f.in_use {
            let mut name_len = 0;
            while f.name[name_len] != 0 && name_len < 32 {
                name_len += 1;
            }
            if offset + name_len + 16 > buf_size as usize {
                break;
            }
            core::ptr::copy_nonoverlapping(f.name.as_ptr(), buf.add(offset), name_len);
            offset += name_len;
            buf.add(offset).write(b' ');
            offset += 1;

            let mut len_val = f.len;
            if len_val == 0 {
                buf.add(offset).write(b'0');
                offset += 1;
            } else {
                let mut digits = [0u8; 10];
                let mut d_count = 0;
                while len_val > 0 {
                    digits[d_count] = b'0' + (len_val % 10) as u8;
                    len_val /= 10;
                    d_count += 1;
                }
                while d_count > 0 {
                    d_count -= 1;
                    buf.add(offset).write(digits[d_count]);
                    offset += 1;
                }
            }
            buf.add(offset).write(b'\n');
            offset += 1;
        }
    }
    offset as i32
}

#[cfg(test)]
pub unsafe fn sys_write_file(filename: *const u8, data: *const u8, size: u32) -> i32 {
    let mut len = 0;
    while *filename.add(len) != 0 && len < 64 {
        len += 1;
    }
    let query = core::slice::from_raw_parts(filename, len);
    for f in MOCK_VFS_FILES.iter_mut() {
        if f.in_use {
            let mut f_len = 0;
            while f.name[f_len] != 0 && f_len < 32 {
                f_len += 1;
            }
            if &f.name[..f_len] == query {
                let to_copy = (size as usize).min(1024);
                core::ptr::copy_nonoverlapping(data, f.data.as_mut_ptr(), to_copy);
                f.len = to_copy;
                return 1;
            }
        }
    }
    for f in MOCK_VFS_FILES.iter_mut() {
        if !f.in_use {
            f.in_use = true;
            let n = query.len().min(31);
            f.name[..n].copy_from_slice(&query[..n]);
            f.name[n] = 0;
            let to_copy = (size as usize).min(1024);
            core::ptr::copy_nonoverlapping(data, f.data.as_mut_ptr(), to_copy);
            f.len = to_copy;
            MOCK_VFS_FILE_COUNT += 1;
            return 1;
        }
    }
    -1
}

#[cfg(test)]
pub unsafe fn sys_delete_file(filename: *const u8) -> i32 {
    let mut len = 0;
    while *filename.add(len) != 0 && len < 64 {
        len += 1;
    }
    let query = core::slice::from_raw_parts(filename, len);
    for f in MOCK_VFS_FILES.iter_mut() {
        if f.in_use {
            let mut f_len = 0;
            while f.name[f_len] != 0 && f_len < 32 {
                f_len += 1;
            }
            if &f.name[..f_len] == query {
                f.in_use = false;
                f.len = 0;
                if MOCK_VFS_FILE_COUNT > 0 {
                    MOCK_VFS_FILE_COUNT -= 1;
                }
                return 1;
            }
        }
    }
    -1
}

#[cfg(test)]
pub unsafe fn sys_pkgrepo_ctl(cmd: u32, arg: *mut u8) -> i32 {
    match cmd {
        NOVA_PKGREPO_CTL_STATUS => {
            if !arg.is_null() {
                let out_ptr = arg as *mut NovaPkgrepoStatus;
                *out_ptr = MOCK_PKGREPO_STATUS;
            }
            0
        }
        NOVA_PKGREPO_CTL_START => {
            MOCK_PKGREPO_STATUS.is_running = 1;
            0
        }
        NOVA_PKGREPO_CTL_STOP => {
            MOCK_PKGREPO_STATUS.is_running = 0;
            0
        }
        _ => -1,
    }
}

#[cfg(test)]
pub unsafe fn sys_exec(_path: *const u8, _argv: *const *const u8, _argc: i32) -> i32 { 101 }
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
