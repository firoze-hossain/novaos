//! userland/desktop/nova-compositor/src/ffi.rs
//!
//! NovaOS System Call ABI Wrappers for Wayland Display Server
//!
//! Bridges userland compositor primitives to kernel system calls:
//!   - `int 0x80` syscall dispatch on 32-bit x86.
//!   - DRM character device (`/dev/dri/card0`) IOCTL dispatch (`sys_ioctl`).
//!   - DMA-BUF zero-copy memory management (`SYS_DMABUF_*`).
//!   - Framebuffer ownership handshake (`SYS_FB_ACQUIRE`, `SYS_FB_RELEASE`, `SYS_FB_PRESENT`).

#![allow(dead_code)]

#[cfg(not(test))]
extern "C" {
    pub fn sys_write(msg: *const u8) -> i32;
    pub fn sys_yield() -> i32;
    pub fn sys_exit(code: i32) -> !;
    pub fn sys_getuid() -> u32;
    pub fn sys_getpid() -> u32;
    pub fn sys_open(path: *const u8, flags: i32) -> i32;
    pub fn sys_close(fd: i32) -> i32;
    pub fn sys_ioctl(fd: i32, cmd: u32, arg: *mut u8) -> i32;
    pub fn sys_read(fd: i32, buf: *mut u8, len: u32) -> i32;

    pub fn sys_fb_acquire() -> i32;
    pub fn sys_fb_release() -> i32;
    pub fn sys_fb_present(buf: *const u32, w: u32, h: u32, pitch: u32) -> i32;

    // DMA-BUF Syscall ABI
    pub fn sys_dmabuf_alloc(args: *mut u8) -> i32;
    pub fn sys_dmabuf_export(args: *mut u8) -> i32;
    pub fn sys_dmabuf_import(args: *mut u8) -> i32;
    pub fn sys_dmabuf_map(args: *mut u8) -> i32;
    pub fn sys_dmabuf_unmap(args: *mut u8) -> i32;
    pub fn sys_dmabuf_free(args: *mut u8) -> i32;
    pub fn sys_dmabuf_sync(args: *mut u8) -> i32;
}

#[cfg(test)]
pub unsafe fn sys_write(_msg: *const u8) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_yield() -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_exit(_code: i32) -> ! { panic!("sys_exit called in test") }
#[cfg(test)]
pub unsafe fn sys_getuid() -> u32 { 0 }
#[cfg(test)]
pub unsafe fn sys_getpid() -> u32 { 1 }
#[cfg(test)]
pub unsafe fn sys_open(_path: *const u8, _flags: i32) -> i32 { 3 }
#[cfg(test)]
pub unsafe fn sys_close(_fd: i32) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_ioctl(_fd: i32, _cmd: u32, _arg: *mut u8) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_read(_fd: i32, _buf: *mut u8, _len: u32) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_acquire() -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_release() -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_present(_buf: *const u32, _w: u32, _h: u32, _pitch: u32) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_dmabuf_alloc(_args: *mut u8) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_dmabuf_export(_args: *mut u8) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_dmabuf_import(_args: *mut u8) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_dmabuf_map(_args: *mut u8) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_dmabuf_unmap(_args: *mut u8) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_dmabuf_free(_args: *mut u8) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_dmabuf_sync(_args: *mut u8) -> i32 { 0 }
