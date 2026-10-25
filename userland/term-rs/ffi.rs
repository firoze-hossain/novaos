//! userland/term-rs/ffi.rs - FFI bindings to NovaOS syscalls (VFS, graphics, mouse, RTC, exec).

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

    // Real-Time Clock
    pub fn sys_rtc_read(out: *mut RtcTime) -> i32;

    // Process & Execution
    pub fn sys_exec(filename: *const u8, argv: *const *const u8, argc: i32) -> i32;
    pub fn sys_exec_trusted(filename: *const u8, argv: *const *const u8, argc: i32) -> i32;
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
        let msg = b"NovaOS Mock File Content\n";
        let copy_len = msg.len().min(max_len as usize);
        core::ptr::copy_nonoverlapping(msg.as_ptr(), buf, copy_len);
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
        let mock_listing = b"HELLO.TXT|SHELL.ELF|SYSTEM.CFG|";
        let len = mock_listing.len().min(buf_size as usize);
        core::ptr::copy_nonoverlapping(mock_listing.as_ptr(), buf, len);
        len as i32
    } else {
        0
    }
}
#[cfg(test)]
pub unsafe fn sys_write_file(_filename: *const u8, _data: *const u8, _size: u32) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_delete_file(_filename: *const u8) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_rtc_read(out: *mut RtcTime) -> i32 {
    if !out.is_null() {
        *out = RtcTime {
            year: 2026,
            month: 10,
            day: 25,
            hour: 21,
            minute: 0,
            second: 0,
        };
    }
    0
}
#[cfg(test)]
pub unsafe fn sys_exec(_filename: *const u8, _argv: *const *const u8, _argc: i32) -> i32 { 42 }
#[cfg(test)]
pub unsafe fn sys_exec_trusted(_filename: *const u8, _argv: *const *const u8, _argc: i32) -> i32 { 42 }
#[cfg(test)]
pub unsafe fn sys_wait(_pid: i32) -> i32 { 0 }
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
