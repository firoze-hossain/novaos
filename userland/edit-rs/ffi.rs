//! userland/edit-rs/ffi.rs - FFI bindings to NovaOS syscalls (VFS, graphics, mouse).

#![allow(dead_code)]

/// Matches kernel/drivers/mouse/ps2mouse.h's mouse_state_t.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MouseState {
    pub dx: i32,
    pub dy: i32,
    pub left_button: bool,
    pub right_button: bool,
    pub middle_button: bool,
}

/// Matches nova_fb_abi.h's nova_fb_info_t (36 bytes).
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

/// Matches nova_fb_abi.h's nova_fb_create_t (28 bytes).
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

/// Matches nova_fb_abi.h's nova_fb_present_t (32 bytes).
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

    // Process & Execution
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
pub static mut MOCK_VFS_FILE_DATA: [u8; 4096] = [0; 4096];
#[cfg(test)]
pub static mut MOCK_VFS_FILE_LEN: usize = 0;
#[cfg(test)]
pub static mut MOCK_VFS_FILE_NAME: [u8; 64] = [0; 64];

#[cfg(test)]
pub unsafe fn reset_mock_vfs() {
    let sample = b"fn main() {\n    let msg = \"Hello from NovaOS Editor!\";\n    // Line 3 comment\n}\n";
    core::ptr::copy_nonoverlapping(sample.as_ptr(), core::ptr::addr_of_mut!(MOCK_VFS_FILE_DATA) as *mut u8, sample.len());
    MOCK_VFS_FILE_LEN = sample.len();
    let name = b"HELLO.TXT\0";
    core::ptr::copy_nonoverlapping(name.as_ptr(), core::ptr::addr_of_mut!(MOCK_VFS_FILE_NAME) as *mut u8, name.len());
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
pub unsafe fn sys_open(_filename: *const u8) -> i32 { 1 }

#[cfg(test)]
pub unsafe fn sys_read(_handle: i32, buf: *mut u8, max_len: i32) -> i32 {
    if !buf.is_null() && max_len > 0 {
        if MOCK_VFS_FILE_LEN == 0 {
            reset_mock_vfs();
        }
        let copy_len = MOCK_VFS_FILE_LEN.min(max_len as usize);
        core::ptr::copy_nonoverlapping(core::ptr::addr_of!(MOCK_VFS_FILE_DATA) as *const u8, buf, copy_len);
        copy_len as i32
    } else {
        0
    }
}

#[cfg(test)]
pub unsafe fn sys_close(_handle: i32) {}

#[cfg(test)]
pub unsafe fn sys_list_files(buf: *mut u8, buf_size: i32) -> i32 {
    if !buf.is_null() && buf_size >= 40 {
        let mock_listing = b"HELLO.TXT|SCRATCH.TXT|SYSTEM.CFG|";
        let len = mock_listing.len().min(buf_size as usize);
        core::ptr::copy_nonoverlapping(mock_listing.as_ptr(), buf, len);
        len as i32
    } else {
        0
    }
}

#[cfg(test)]
pub unsafe fn sys_write_file(_filename: *const u8, data: *const u8, size: u32) -> i32 {
    if !data.is_null() && size <= 4096 {
        core::ptr::copy_nonoverlapping(data, core::ptr::addr_of_mut!(MOCK_VFS_FILE_DATA) as *mut u8, size as usize);
        MOCK_VFS_FILE_LEN = size as usize;
        0
    } else {
        -1
    }
}

#[cfg(test)]
pub unsafe fn sys_delete_file(_filename: *const u8) -> i32 { 0 }

#[cfg(test)]
pub unsafe fn sys_exit(_code: i32) -> ! {
    panic!("sys_exit called in test mode");
}

#[cfg(test)]
pub unsafe fn sys_fb_info(out: *mut NovaFbInfo) -> i32 {
    if !out.is_null() {
        *out = NovaFbInfo {
            struct_size: core::mem::size_of::<NovaFbInfo>() as u32,
            width: 1024,
            height: 768,
            format: 0,
            backend: 1,
            backend_mask: 1,
            caps: 0,
            max_surface_dim: 2048,
            max_surfaces: 16,
        };
    }
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_acquire(_backend: u32) -> i32 { 0 }

#[cfg(test)]
pub unsafe fn sys_fb_release() -> i32 { 0 }

#[cfg(test)]
pub unsafe fn sys_fb_create(req: *mut NovaFbCreate) -> i32 {
    if !req.is_null() {
        static mut TEST_PIXELS: [u32; 1024 * 768] = [0; 1024 * 768];
        (*req).handle = 1;
        (*req).pixels = core::ptr::addr_of_mut!(TEST_PIXELS) as *mut u32 as u32;
        (*req).stride = 1024 * 4;
        (*req).size = 1024 * 768 * 4;
    }
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_present(_req: *const NovaFbPresent) -> i32 { 0 }

#[cfg(test)]
pub unsafe fn sys_fb_damage(_handle: u32, _x: i32, _y: i32, _w: i32, _h: i32) -> i32 { 0 }
