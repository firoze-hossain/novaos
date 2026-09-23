//! userland/wm-rs/ffi.rs - FFI bindings to NovaOS's existing,
//! unmodified C syscall wrapper layer (userland/libc/syscall.c) - the
//! same "binding layer, not new functionality" approach userland/
//! ping-rs/ffi.rs and userland/coreutils-rs/ffi.rs already established
//! (Phase 33, Phase 59). Every function here already exists and works
//! (Phase 32b's SYS_GFX_*/SYS_MOUSE_READ, proven by userland/
//! coreutils/gui.c's own static demo) - this window manager is new
//! logic built entirely on top of already-real syscalls, not new
//! kernel surface.

#![allow(dead_code)]

/// Matches kernel/drivers/mouse/ps2mouse.h's own mouse_state_t
/// exactly (see userland/libc/include/novasys.h's own identical
/// struct and its comment on why this layout match is safe: neither
/// side uses `#[repr(packed)]`, so both need the same compiler's
/// default alignment rules, which `-m32` on the C side and this
/// kernel's own i686 Rust target on this side both guarantee).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MouseState {
    pub dx: i32,
    pub dy: i32,
    pub left_button: bool,
    pub right_button: bool,
    pub middle_button: bool,
}

extern "C" {
    /// Writes a NUL-terminated string to the console/debug log.
    pub fn sys_write(s: *const u8) -> i32;

    /// Yields the remaining timeslice back to the scheduler.
    pub fn sys_yield();

    /// Reads one buffered keypress as a real ASCII character, or -1
    /// if nothing is buffered right now - a non-blocking poll, not a
    /// wait (see kernel/arch/x86/cpu/syscall.c's own handle_read_key()
    /// - it only ever calls the keyboard driver's own blocking get-
    /// char function after first confirming a character is already
    /// available).
    pub fn sys_read_key() -> i32;

    /// Enters VGA Mode 13h (320x200, 256 colors). No return value -
    /// this either works or the whole system has bigger problems.
    pub fn sys_gfx_enter();

    /// Restores text mode and clears it.
    pub fn sys_gfx_exit();

    /// Sets one pixel. Silently clipped if (x, y) is off-screen -
    /// see kernel-side vga_put_pixel()'s own bounds check.
    pub fn sys_gfx_put_pixel(x: i32, y: i32, color: u8);

    /// Fills a w*h rectangle starting at (x, y) with one solid color.
    pub fn sys_gfx_fill_rect(x: i32, y: i32, w: i32, h: i32, color: u8);

    /// Reads the current mouse delta/button state since the last
    /// read. Returns 1 if a real PS/2 mouse is present, 0 if not
    /// (`*out` is left zeroed either way, so a caller that doesn't
    /// check the return value still gets harmless, inert input rather
    /// than garbage).
    pub fn sys_mouse_read(out: *mut MouseState) -> i32;
}
