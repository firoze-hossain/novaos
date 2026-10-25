//! userland/settings-rs/ffi.rs - FFI bindings to NovaOS syscalls (VFS, graphics, mouse).

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

    // Framebuffer graphics API
    pub fn sys_fb_info(out: *mut NovaFbInfo) -> i32;
    pub fn sys_fb_acquire(backend: u32) -> i32;
    pub fn sys_fb_release() -> i32;
    pub fn sys_fb_create(req: *mut NovaFbCreate) -> i32;
    pub fn sys_fb_destroy(handle: u32) -> i32;
    pub fn sys_fb_present(req: *const NovaFbPresent) -> i32;
}

#[cfg(test)]
pub unsafe fn sys_write(_s: *const u8) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_yield() {}
#[cfg(test)]
pub unsafe fn sys_read_key() -> i32 { -1 }
#[cfg(test)]
pub unsafe fn sys_mouse_read(_out: *mut MouseState) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_open(_filename: *const u8) -> i32 { 3 }
#[cfg(test)]
pub unsafe fn sys_read(_handle: i32, buf: *mut u8, max_len: i32) -> i32 {
    let dummy = b"NVCFnovaos\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0firoze\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0SHELL.ELF\0\0\0\0";
    let len = dummy.len().min(max_len as usize);
    if !buf.is_null() && len > 0 {
        buf.copy_from_nonoverlapping(dummy.as_ptr(), len);
    }
    len as i32
}
#[cfg(test)]
pub unsafe fn sys_close(_handle: i32) {}
#[cfg(test)]
pub unsafe fn sys_list_files(buf: *mut u8, buf_size: i32) -> i32 {
    let dummy = b"SYSTEM.CFG 81\nUSERS.CFG 716\nSERVICES.CFG 160\nFIREWALL.CFG 512\n";
    let len = dummy.len().min(buf_size as usize);
    if !buf.is_null() && len > 0 {
        buf.copy_from_nonoverlapping(dummy.as_ptr(), len);
    }
    len as i32
}
#[cfg(test)]
pub unsafe fn sys_write_file(_filename: *const u8, _data: *const u8, _size: u32) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_delete_file(_filename: *const u8) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_info(_out: *mut NovaFbInfo) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_acquire(_backend: u32) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_release() -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_create(_req: *mut NovaFbCreate) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_destroy(_handle: u32) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_present(_req: *const NovaFbPresent) -> i32 { 0 }
