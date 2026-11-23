//! userland/screenshot-rs/ffi.rs - FFI bindings for NovaOS screenshot utility.

#![allow(dead_code, static_mut_refs)]

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
pub struct NovaFbReadback {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub dst: u32,
    pub dst_stride: u32,
    pub flags: u32,
}

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

#[cfg(not(test))]
extern "C" {
    pub fn sys_write(s: *const u8) -> i32;
    pub fn sys_exit(code: i32) -> !;
    pub fn sys_yield();
    pub fn sys_fb_info(out: *mut NovaFbInfo) -> i32;
    pub fn sys_fb_acquire(backend: u32) -> i32;
    pub fn sys_fb_release() -> i32;
    pub fn sys_fb_readback(req: *const NovaFbReadback) -> i32;
    pub fn sys_rtc_read(out: *mut RtcTime) -> i32;
    pub fn sys_write_file(filename: *const u8, data: *const u8, size: u32) -> i32;
    pub fn sys_open(filename: *const u8) -> i32;
    pub fn sys_read(handle: i32, buf: *mut u8, max_len: i32) -> i32;
    pub fn sys_close(handle: i32);
}

#[cfg(test)]
pub unsafe fn sys_write(_s: *const u8) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_exit(_code: i32) -> ! { loop {} }
#[cfg(test)]
pub unsafe fn sys_yield() {}
#[cfg(test)]
pub unsafe fn sys_fb_info(out: *mut NovaFbInfo) -> i32 {
    if !out.is_null() {
        (*out).width = 1024;
        (*out).height = 768;
    }
    0
}
#[cfg(test)]
pub unsafe fn sys_fb_acquire(_backend: u32) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_release() -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_readback(_req: *const NovaFbReadback) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_rtc_read(out: *mut RtcTime) -> i32 {
    if !out.is_null() {
        *out = RtcTime { year: 2026, month: 11, day: 23, hour: 7, minute: 30, second: 0 };
    }
    0
}
#[cfg(test)]
pub unsafe fn sys_write_file(_filename: *const u8, _data: *const u8, size: u32) -> i32 { size as i32 }
#[cfg(test)]
pub unsafe fn sys_open(_filename: *const u8) -> i32 { -1 }
#[cfg(test)]
pub unsafe fn sys_read(_handle: i32, _buf: *mut u8, _max_len: i32) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_close(_handle: i32) {}
