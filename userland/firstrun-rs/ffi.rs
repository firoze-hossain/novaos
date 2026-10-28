//! userland/firstrun-rs/ffi.rs - FFI bindings and mock environment for NovaOS First-Run Setup Wizard.
//! Interfaces with NovaOS ring-3 system calls (VFS, Framebuffer, Mouse, Process Telemetry).

#![allow(dead_code, static_mut_refs)]

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

pub const NOVA_PROC_OP_GET_SYSINFO: u32 = 1;

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NovaSysinfo {
    pub total_ram_pages: u32,
    pub free_ram_pages: u32,
    pub process_count: u32,
    pub cpu_count: u32,
    pub uptime_ticks: u32,
}

// ===========================================================================
// Bare-Metal Syscalls (not(test))
// ===========================================================================

#[cfg(not(test))]
extern "C" {
    pub fn sys_write(s: *const u8) -> i32;
    pub fn sys_exit(code: i32) -> !;
    pub fn sys_yield();
    pub fn sys_read_key() -> i32;
    pub fn sys_mouse_read(out: *mut MouseState) -> i32;

    pub fn sys_open(filename: *const u8) -> i32;
    pub fn sys_read(handle: i32, buf: *mut u8, max_len: i32) -> i32;
    pub fn sys_close(handle: i32);
    pub fn sys_write_file(filename: *const u8, data: *const u8, size: u32) -> i32;
    pub fn sys_delete_file(filename: *const u8) -> i32;

    pub fn sys_fb_info(out: *mut NovaFbInfo) -> i32;
    pub fn sys_fb_acquire(backend: u32) -> i32;
    pub fn sys_fb_release() -> i32;
    pub fn sys_fb_create(req: *mut NovaFbCreate) -> i32;
    pub fn sys_fb_destroy(handle: u32) -> i32;
    pub fn sys_fb_present(req: *const NovaFbPresent) -> i32;

    pub fn sys_proc_info(op: u32, arg0: u32, buf: *mut u8, len: u32) -> i32;
}

// ===========================================================================
// Host Unit Testing Mocks (cfg(test))
// ===========================================================================

#[cfg(test)]
pub struct MockFile {
    pub name: [u8; 32],
    pub data: [u8; 2048],
    pub len: usize,
    pub in_use: bool,
}

#[cfg(test)]
impl MockFile {
    pub const fn empty() -> Self {
        MockFile {
            name: [0; 32],
            data: [0; 2048],
            len: 0,
            in_use: false,
        }
    }
}

#[cfg(test)]
pub static mut MOCK_FILES: [MockFile; 8] = [
    MockFile::empty(),
    MockFile::empty(),
    MockFile::empty(),
    MockFile::empty(),
    MockFile::empty(),
    MockFile::empty(),
    MockFile::empty(),
    MockFile::empty(),
];

#[cfg(test)]
pub static mut MOCK_SYSINFO: NovaSysinfo = NovaSysinfo {
    total_ram_pages: 131072, // 512 MB
    free_ram_pages: 120000,
    process_count: 8,
    cpu_count: 2,
    uptime_ticks: 450,
};

#[cfg(test)]
pub static mut MOCK_LAST_WRITTEN_FILE: [u8; 32] = [0; 32];
#[cfg(test)]
pub static mut MOCK_LAST_WRITTEN_SIZE: usize = 0;

#[cfg(test)]
pub unsafe fn reset_mock_environment() {
    for f in MOCK_FILES.iter_mut() {
        *f = MockFile::empty();
    }
    MOCK_LAST_WRITTEN_FILE = [0; 32];
    MOCK_LAST_WRITTEN_SIZE = 0;
}

#[cfg(test)]
pub unsafe fn sys_write(_s: *const u8) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_exit(_code: i32) -> ! {
    panic!("sys_exit called in test mode");
}

#[cfg(test)]
pub unsafe fn sys_yield() {}

#[cfg(test)]
pub unsafe fn sys_read_key() -> i32 {
    -1
}

#[cfg(test)]
pub unsafe fn sys_mouse_read(_out: *mut MouseState) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_open(filename: *const u8) -> i32 {
    if filename.is_null() {
        return -14;
    }
    let mut len = 0;
    while *filename.add(len) != 0 && len < 32 {
        len += 1;
    }
    let target = core::slice::from_raw_parts(filename, len);
    for (i, f) in MOCK_FILES.iter().enumerate() {
        if f.in_use && &f.name[..len] == target && f.name[len] == 0 {
            return i as i32;
        }
    }
    -1
}

#[cfg(test)]
pub unsafe fn sys_read(handle: i32, buf: *mut u8, max_len: i32) -> i32 {
    if handle < 0 || handle >= 8 || buf.is_null() || max_len <= 0 {
        return -1;
    }
    let file = &MOCK_FILES[handle as usize];
    if !file.in_use {
        return -1;
    }
    let read_len = file.len.min(max_len as usize);
    core::ptr::copy_nonoverlapping(file.data.as_ptr(), buf, read_len);
    read_len as i32
}

#[cfg(test)]
pub unsafe fn sys_close(_handle: i32) {}

#[cfg(test)]
pub unsafe fn sys_write_file(filename: *const u8, data: *const u8, size: u32) -> i32 {
    if filename.is_null() || (size > 0 && data.is_null()) {
        return -14;
    }
    let mut len = 0;
    while *filename.add(len) != 0 && len < 31 {
        len += 1;
    }
    let target = core::slice::from_raw_parts(filename, len);

    // Track last written
    MOCK_LAST_WRITTEN_FILE = [0; 32];
    MOCK_LAST_WRITTEN_FILE[..len].copy_from_slice(target);
    MOCK_LAST_WRITTEN_SIZE = size as usize;

    let mut slot = None;
    for (i, f) in MOCK_FILES.iter_mut().enumerate() {
        if f.in_use && &f.name[..len] == target && f.name[len] == 0 {
            slot = Some(i);
            break;
        }
    }
    if slot.is_none() {
        for (i, f) in MOCK_FILES.iter().enumerate() {
            if !f.in_use {
                slot = Some(i);
                break;
            }
        }
    }
    let idx = slot.unwrap_or(0);
    let mf = &mut MOCK_FILES[idx];
    mf.in_use = true;
    mf.name = [0; 32];
    mf.name[..len].copy_from_slice(target);
    let copy_size = (size as usize).min(2048);
    if copy_size > 0 {
        core::ptr::copy_nonoverlapping(data, mf.data.as_mut_ptr(), copy_size);
    }
    mf.len = copy_size;
    copy_size as i32
}

#[cfg(test)]
pub unsafe fn sys_delete_file(filename: *const u8) -> i32 {
    if filename.is_null() {
        return -14;
    }
    let mut len = 0;
    while *filename.add(len) != 0 && len < 31 {
        len += 1;
    }
    let target = core::slice::from_raw_parts(filename, len);
    for f in MOCK_FILES.iter_mut() {
        if f.in_use && &f.name[..len] == target && f.name[len] == 0 {
            *f = MockFile::empty();
            return 1;
        }
    }
    -1
}

#[cfg(test)]
pub unsafe fn sys_fb_info(out: *mut NovaFbInfo) -> i32 {
    if out.is_null() {
        return -14;
    }
    (*out).width = 1024;
    (*out).height = 768;
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
    if req.is_null() {
        return -14;
    }
    (*req).handle = 1;
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
pub unsafe fn sys_proc_info(op: u32, _arg0: u32, buf: *mut u8, len: u32) -> i32 {
    if op == NOVA_PROC_OP_GET_SYSINFO {
        if buf.is_null() || len < core::mem::size_of::<NovaSysinfo>() as u32 {
            return -14;
        }
        core::ptr::copy_nonoverlapping(
            &MOCK_SYSINFO as *const _ as *const u8,
            buf,
            core::mem::size_of::<NovaSysinfo>(),
        );
        return 0;
    }
    -1
}
