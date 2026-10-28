//! userland/wm-rs/ffi.rs - FFI bindings to NovaOS's framebuffer graphics
//! syscalls (SYS_FB_*), mouse polling, and system calls.

#![allow(dead_code)]

/// Matches kernel/drivers/mouse/ps2mouse.h's own mouse_state_t exactly.
#[repr(C)]
#[derive(Clone, Copy)]
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

/// Matches nova_fb_abi.h's nova_fb_readback_t (28 bytes).
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct NovaFbReadback {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub dst: u32,
    pub dst_stride: u32,
    pub flags: u32,
}

pub const NOVA_FB_BACKEND_ANY: u32 = 0;
pub const NOVA_FB_BACKEND_VBE: u32 = 1;
pub const NOVA_FB_BACKEND_VIRTIOGPU: u32 = 2;

#[cfg(not(test))]
extern "C" {
    /// Writes a NUL-terminated string to the console/debug log.
    pub fn sys_write(s: *const u8) -> i32;

    /// Yields the remaining timeslice back to the scheduler.
    pub fn sys_yield();

    /// Reads one buffered keypress as an ASCII character, or -1 if none buffered.
    pub fn sys_read_key() -> i32;

    /// Reads current mouse delta and button states.
    pub fn sys_mouse_read(out: *mut MouseState) -> i32;

    // --- Modern Framebuffer Graphics API (SYS_FB_*) ---

    /// Queries framebuffer display geometry and capabilities.
    pub fn sys_fb_info(out: *mut NovaFbInfo) -> i32;

    /// Takes exclusive ownership of the display and clears to black.
    pub fn sys_fb_acquire(backend: u32) -> i32;

    /// Releases display back to the system (restoring text console).
    pub fn sys_fb_release() -> i32;

    /// Creates a 32-bit XRGB memory surface in caller's address space.
    pub fn sys_fb_create(req: *mut NovaFbCreate) -> i32;

    /// Destroys a surface and unmaps its memory.
    pub fn sys_fb_destroy(handle: u32) -> i32;

    /// Presents a damaged rectangle from surface to display.
    pub fn sys_fb_present(req: *const NovaFbPresent) -> i32;

    /// Reads back display content into memory buffer.
    pub fn sys_fb_readback(req: *const NovaFbReadback) -> i32;

    // --- Legacy VGA Mode Calls (kept for fallback) ---
    pub fn sys_gfx_enter();
    pub fn sys_gfx_exit();
    pub fn sys_gfx_put_pixel(x: i32, y: i32, color: u8);
    pub fn sys_gfx_fill_rect(x: i32, y: i32, w: i32, h: i32, color: u8);

    // --- VFS File Syscalls ---
    pub fn sys_open(filename: *const u8) -> i32;
    pub fn sys_read(handle: i32, buf: *mut u8, max_len: i32) -> i32;
    pub fn sys_close(handle: i32);
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
#[cfg(test)]
pub unsafe fn sys_fb_readback(_req: *const NovaFbReadback) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_gfx_enter() {}
#[cfg(test)]
pub unsafe fn sys_gfx_exit() {}
#[cfg(test)]
pub unsafe fn sys_gfx_put_pixel(_x: i32, _y: i32, _color: u8) {}
#[cfg(test)]
pub unsafe fn sys_gfx_fill_rect(_x: i32, _y: i32, _w: i32, _h: i32, _color: u8) {}
#[cfg(test)]
pub unsafe fn sys_open(_filename: *const u8) -> i32 { -1 }
#[cfg(test)]
pub unsafe fn sys_read(_handle: i32, _buf: *mut u8, _max_len: i32) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_close(_handle: i32) {}
