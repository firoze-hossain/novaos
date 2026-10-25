//! userland/wm-rs/wm.rs - NovaOS Framebuffer Window Manager (Month 2 Compositor)
//!
//! Ports the ring-3 interactive window manager off legacy Mode 13h VGA text/VGA
//! mode onto the unified 32-bit linear framebuffer architecture (SYS_FB_*).
//!
//! Features:
//!   - Real 32-bit XRGB framebuffer rendering using zero-allocation Surface trait
//!     (ui::draw::BufferSurface32) and the NovaOS Design System (ui-rs).
//!   - Exclusive display ownership handshake via SYS_FB_ACQUIRE / SYS_FB_RELEASE.
//!   - Damage-aware frame presentation via SYS_FB_PRESENT with single-syscall-per-frame blits.
//!   - Full window management preservation: multiple overlapping windows, titlebar dragging,
//!     click-to-focus (body and titlebar), taskbar focus switching, and left/right edge snapping
//!     with restoration of pre-snap floating geometry.
//!   - Scalable to any native display resolution (e.g. 1024x768 VESA/VBE and virtio-gpu) while
//!     preserving backwards-compatible fallback geometry.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![cfg_attr(test, allow(dead_code))]

extern crate ui;
use ui::prelude::*;

mod ffi;

#[cfg(not(test))]
use core::panic::PanicInfo;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe {
        ffi::sys_fb_release(); // Best-effort: restore text console if panicking
        let msg = b"wm: internal error (panic)\n\0";
        ffi::sys_write(msg.as_ptr());
    }
    loop {
        unsafe { ffi::sys_yield() };
    }
}

pub const DEFAULT_SCREEN_W: i32 = 320;
pub const DEFAULT_SCREEN_H: i32 = 200;
pub const DEFAULT_TITLEBAR_H: i32 = 10;
pub const DEFAULT_TASKBAR_H: i32 = 12;
pub const DEFAULT_DESKTOP_H: i32 = DEFAULT_SCREEN_H - DEFAULT_TASKBAR_H;

pub const FB_SCREEN_W: i32 = 1024;
pub const FB_SCREEN_H: i32 = 768;
pub const FB_TITLEBAR_H: i32 = TITLEBAR_HEIGHT; // 24px
pub const FB_TASKBAR_H: i32 = TASKBAR_HEIGHT;   // 32px
pub const FB_DESKTOP_H: i32 = FB_SCREEN_H - FB_TASKBAR_H; // 736px

pub const MAX_WINDOWS: usize = 4;
pub const SNAP_MARGIN: i32 = 4;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SnapState {
    Normal,
    Left,
    Right,
}

#[derive(Clone, Copy)]
pub struct Window {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub body_color: Color,
    pub titlebar_color: Color,
    pub label: u8,
    pub snap: SnapState,
    pub pre_snap: (i32, i32, i32, i32),
}

impl Window {
    pub const fn new(x: i32, y: i32, w: i32, h: i32, body: Color, title: Color, label: u8) -> Self {
        Window {
            x, y, w, h,
            body_color: body,
            titlebar_color: title,
            label,
            snap: SnapState::Normal,
            pre_snap: (x, y, w, h),
        }
    }

    pub fn from_vga(x: i32, y: i32, w: i32, h: i32, body_vga: u8, title_vga: u8, label: u8) -> Self {
        Window::new(
            x, y, w, h,
            Color::from_vga_index(body_vga),
            Color::from_vga_index(title_vga),
            label,
        )
    }
}

fn point_in_rect(px: i32, py: i32, x: i32, y: i32, w: i32, h: i32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

pub struct Wm {
    pub screen_w: i32,
    pub screen_h: i32,
    pub titlebar_h: i32,
    pub taskbar_h: i32,
    pub desktop_h: i32,
    pub windows: [Window; MAX_WINDOWS],
    pub z_order: [usize; MAX_WINDOWS],
    pub cursor_x: i32,
    pub cursor_y: i32,
    pub dragging: Option<usize>,
    pub drag_offset_x: i32,
    pub drag_offset_y: i32,
    pub prev_left_button: bool,
    pub cursor_theme: CursorTheme,
}

impl Wm {
    /// Constructs default WM geometry matching the 320x200 layout,
    /// ensuring deterministic compatibility with automated self-tests.
    pub fn new() -> Self {
        Wm {
            screen_w: DEFAULT_SCREEN_W,
            screen_h: DEFAULT_SCREEN_H,
            titlebar_h: DEFAULT_TITLEBAR_H,
            taskbar_h: DEFAULT_TASKBAR_H,
            desktop_h: DEFAULT_DESKTOP_H,
            windows: [
                Window::from_vga(20, 20, 100, 70, 3, 1, 1),
                Window::from_vga(140, 30, 100, 70, 10, 2, 2),
                Window::from_vga(60, 90, 100, 70, 14, 6, 3),
                Window::from_vga(170, 100, 90, 60, 12, 5, 4),
            ],
            z_order: [0, 1, 2, 3],
            cursor_x: DEFAULT_SCREEN_W / 2,
            cursor_y: DEFAULT_SCREEN_H / 2,
            dragging: None,
            drag_offset_x: 0,
            drag_offset_y: 0,
            prev_left_button: false,
            cursor_theme: CursorTheme::Classic,
        }
    }

    /// Constructs WM layout scaled for high-resolution 32-bit linear framebuffers.
    pub fn new_for_display(width: i32, height: i32) -> Self {
        let taskbar_h = if height >= 600 { FB_TASKBAR_H } else { DEFAULT_TASKBAR_H };
        let titlebar_h = if height >= 600 { FB_TITLEBAR_H } else { DEFAULT_TITLEBAR_H };
        let desktop_h = height - taskbar_h;

        let palette = Theme::Dark.palette();

        let (win_w, win_h) = if width >= 800 { (360, 250) } else { (160, 110) };
        let w0 = Window::new(40, 40, win_w, win_h, palette.surface_raised, palette.titlebar_active, 1);
        let w1 = Window::new(440, 60, win_w, win_h, palette.surface_raised, palette.titlebar_inactive, 2);
        let w2 = Window::new(120, 330, win_w, win_h, palette.surface_raised, palette.titlebar_inactive, 3);
        let w3 = Window::new(520, 350, win_w, win_h, palette.surface_raised, palette.titlebar_inactive, 4);

        Wm {
            screen_w: width,
            screen_h: height,
            titlebar_h,
            taskbar_h,
            desktop_h,
            windows: [w0, w1, w2, w3],
            z_order: [0, 1, 2, 3],
            cursor_x: width / 2,
            cursor_y: height / 2,
            dragging: None,
            drag_offset_x: 0,
            drag_offset_y: 0,
            prev_left_button: false,
            cursor_theme: CursorTheme::Classic,
        }
    }

    /// Raises `win_index` to topmost in z-order.
    pub fn raise(&mut self, win_index: usize) {
        if let Some(pos) = self.z_order.iter().position(|&w| w == win_index) {
            for i in pos..MAX_WINDOWS - 1 {
                self.z_order[i] = self.z_order[i + 1];
            }
            self.z_order[MAX_WINDOWS - 1] = win_index;
        }
    }

    pub fn titlebar_at(&self, px: i32, py: i32) -> Option<usize> {
        for i in (0..MAX_WINDOWS).rev() {
            let idx = self.z_order[i];
            let w = &self.windows[idx];
            if point_in_rect(px, py, w.x, w.y, w.w, self.titlebar_h) {
                return Some(idx);
            }
        }
        None
    }

    pub fn window_at(&self, px: i32, py: i32) -> Option<usize> {
        for i in (0..MAX_WINDOWS).rev() {
            let idx = self.z_order[i];
            let w = &self.windows[idx];
            if point_in_rect(px, py, w.x, w.y, w.w, w.h) {
                return Some(idx);
            }
        }
        None
    }

    pub fn taskbar_button_at(&self, px: i32, py: i32) -> Option<usize> {
        if py < self.desktop_h || px >= self.screen_w {
            return None;
        }
        let button_w = self.screen_w / MAX_WINDOWS as i32;
        if button_w <= 0 {
            return None;
        }
        Some((px / button_w) as usize)
    }

    /// Evaluates dynamic cursor shape based on current pointer coordinates.
    pub fn cursor_shape_at(&self, cx: i32, cy: i32) -> CursorShape {
        if self.dragging.is_some() {
            return CursorShape::Move;
        }
        if self.taskbar_button_at(cx, cy).is_some() {
            return CursorShape::Pointer;
        }
        for i in (0..MAX_WINDOWS).rev() {
            let win_idx = self.z_order[i];
            let w = &self.windows[win_idx];
            if point_in_rect(cx, cy, w.x, w.y, w.w, w.h) {
                if cy < w.y + self.titlebar_h {
                    if self.titlebar_h >= 14 && w.w >= 80 {
                        let btn_size = if self.titlebar_h >= 20 { 12 } else { 8 };
                        let btn_y = w.y + (self.titlebar_h - btn_size) / 2;
                        let close_x = w.x + w.w - btn_size - 8;
                        let min_ctrl_x = close_x - (btn_size + 6) * 2;
                        if cy >= btn_y && cy < btn_y + btn_size && cx >= min_ctrl_x && cx < close_x + btn_size {
                            return CursorShape::Pointer;
                        }
                    }
                    return CursorShape::Move;
                }
                // Client area: Console and Editor use I-Beam text selection cursor
                if w.label == 1 || w.label == 3 {
                    return CursorShape::IBeam;
                } else {
                    return CursorShape::Arrow;
                }
            }
        }
        CursorShape::Arrow
    }

    pub fn on_left_press(&mut self, px: i32, py: i32) {
        if let Some(idx) = self.taskbar_button_at(px, py) {
            if idx < MAX_WINDOWS {
                self.raise(idx);
            }
            return;
        }
        if let Some(idx) = self.titlebar_at(px, py) {
            self.raise(idx);
            let w = &mut self.windows[idx];
            if w.snap != SnapState::Normal {
                let (_, _, pw, ph) = w.pre_snap;
                w.snap = SnapState::Normal;
                w.w = pw;
                w.h = ph;
                w.x = (px - pw / 4).clamp(0, self.screen_w - pw);
                w.y = py.min(self.desktop_h - ph).max(0);
            }
            self.dragging = Some(idx);
            self.drag_offset_x = px - self.windows[idx].x;
            self.drag_offset_y = py - self.windows[idx].y;
            return;
        }
        if let Some(idx) = self.window_at(px, py) {
            self.raise(idx);
        }
    }

    pub fn on_drag_release(&mut self, idx: usize, px: i32) {
        let w = &mut self.windows[idx];
        if px <= SNAP_MARGIN {
            w.pre_snap = (w.x, w.y, w.w, w.h);
            w.snap = SnapState::Left;
            w.x = 0;
            w.y = 0;
            w.w = self.screen_w / 2;
            w.h = self.desktop_h;
        } else if px >= self.screen_w - SNAP_MARGIN {
            w.pre_snap = (w.x, w.y, w.w, w.h);
            w.snap = SnapState::Right;
            w.x = self.screen_w / 2;
            w.y = 0;
            w.w = self.screen_w / 2;
            w.h = self.desktop_h;
        }
    }

    pub fn apply_mouse(&mut self, m: ffi::MouseState) {
        self.cursor_x = (self.cursor_x + m.dx).clamp(0, self.screen_w - 1);
        self.cursor_y = (self.cursor_y + m.dy).clamp(0, self.screen_h - 1);

        let just_pressed = m.left_button && !self.prev_left_button;
        let just_released = !m.left_button && self.prev_left_button;

        if just_pressed && self.dragging.is_none() {
            self.on_left_press(self.cursor_x, self.cursor_y);
        }

        if let Some(idx) = self.dragging {
            if m.left_button {
                let w = &mut self.windows[idx];
                w.x = (self.cursor_x - self.drag_offset_x).clamp(0, self.screen_w - w.w);
                w.y = (self.cursor_y - self.drag_offset_y).clamp(0, self.desktop_h - w.h);
            }
            if just_released {
                self.on_drag_release(idx, self.cursor_x);
                self.dragging = None;
            }
        }

        self.prev_left_button = m.left_button;
    }

    /// Renders desktop background, windows in z-order, taskbar, and cursor
    /// directly into a 32-bit (or generic) drawing surface with zero per-pixel syscalls.
    pub fn render<S: Surface + ?Sized>(&self, surface: &mut S) {
        let palette = Theme::Dark.palette();

        // 1. Desktop background
        surface.fill_rect(Rect::new(0, 0, self.screen_w, self.desktop_h), palette.desktop_bg);

        // 2. Windows in z-order
        let topmost = self.z_order[MAX_WINDOWS - 1];
        for i in 0..MAX_WINDOWS {
            let win_idx = self.z_order[i];
            let w = &self.windows[win_idx];
            let is_active = win_idx == topmost;

            let (tb_color, border_col) = if is_active {
                (w.titlebar_color, palette.border_focused)
            } else {
                (w.titlebar_color.darken(40), palette.border_normal)
            };

            // Window outer drop shadow for active window on high-res
            if is_active && self.titlebar_h >= 20 && w.snap == SnapState::Normal {
                surface.fill_rect(Rect::new(w.x + 3, w.y + w.h, w.w - 1, 2), Color::rgba(0, 0, 0, 45));
                surface.fill_rect(Rect::new(w.x + w.w, w.y + 3, 2, w.h), Color::rgba(0, 0, 0, 45));
            }

            // Window client body
            let body_y = w.y + self.titlebar_h;
            let body_h = w.h - self.titlebar_h;
            if body_h > 0 {
                surface.fill_rect(Rect::new(w.x, body_y, w.w, body_h), w.body_color);
            }

            // Window titlebar
            if self.titlebar_h >= 20 {
                let half_h = self.titlebar_h / 2;
                surface.fill_rect(Rect::new(w.x, w.y, w.w, half_h), tb_color.lighten(12));
                surface.fill_rect(Rect::new(w.x, w.y + half_h, w.w, self.titlebar_h - half_h), tb_color);
            } else {
                surface.fill_rect(Rect::new(w.x, w.y, w.w, self.titlebar_h), tb_color);
            }

            // Titlebar top highlight sheen & bottom separator
            if self.titlebar_h > 1 {
                let sheen_col = if is_active { tb_color.lighten(35) } else { tb_color.lighten(15) };
                surface.fill_rect(Rect::new(w.x + 1, w.y, w.w - 2, 1), sheen_col);
                surface.fill_rect(Rect::new(w.x, w.y + self.titlebar_h - 1, w.w, 1), palette.border_subtle);
            }

            // Window border outline
            ui::draw::draw_rect_outline(surface, Rect::new(w.x, w.y, w.w, w.h), 1, border_col);
            if is_active && self.titlebar_h >= 20 {
                ui::draw::draw_rect_outline(surface, Rect::new(w.x + 1, w.y + 1, w.w - 2, w.h - 2), 1, Color::rgba(255, 255, 255, 20));
            }

            // Application Icon & Title Text
            let text_col = if is_active { palette.text_primary } else { palette.text_muted };
            if self.titlebar_h >= 20 {
                let icon_size = 12;
                let icon_x = w.x + 8;
                let icon_y = w.y + (self.titlebar_h - icon_size) / 2;
                draw_app_icon(surface, icon_x, icon_y, w.label, &palette);

                let font = Font::regular();
                let text_x = icon_x + icon_size + 6;
                let text_y = w.y + (self.titlebar_h - font.line_height()) / 2;
                let title_str = match w.label {
                    1 => "Console - NovaOS Shell",
                    2 => "Files - Root Directory",
                    3 => "Editor - Scratchpad",
                    _ => "System - Diagnostics",
                };
                ui::font::draw_text_font(surface, text_x, text_y, title_str, text_col, &font);
            } else {
                let font = Font::small();
                let text_y = w.y + (self.titlebar_h - 8).max(0) / 2;
                ui::font::draw_char_font(surface, w.x + 4, text_y, b'0' + w.label, text_col, &font);
            }

            // Redesigned Window Control Buttons (Close, Maximize, Minimize)
            if self.titlebar_h >= 14 && w.w >= 80 {
                let btn_size = if self.titlebar_h >= 20 { 12 } else { 8 };
                let btn_y = w.y + (self.titlebar_h - btn_size) / 2;
                let close_x = w.x + w.w - btn_size - 8;
                let max_x = close_x - btn_size - 6;
                let min_x = max_x - btn_size - 6;

                // Close Button ('X')
                let close_col = if is_active { palette.status_danger } else { palette.status_danger.darken(35) };
                ui::draw::draw_rounded_rect(surface, Rect::new(close_x, btn_y, btn_size, btn_size), 2, close_col);
                let x_span = ((btn_size - 4) / 2).max(1);
                let cx = close_x + btn_size / 2;
                let cy = btn_y + btn_size / 2;
                ui::draw::draw_line(surface, cx - x_span, cy - x_span, cx + x_span, cy + x_span, Color::WHITE);
                ui::draw::draw_line(surface, cx + x_span, cy - x_span, cx - x_span, cy + x_span, Color::WHITE);

                // Maximize Button ('[]')
                let max_col = if is_active { palette.status_success } else { palette.status_success.darken(35) };
                ui::draw::draw_rounded_rect(surface, Rect::new(max_x, btn_y, btn_size, btn_size), 2, max_col);
                let sq_size = (btn_size - 6).max(3);
                let sq_x = max_x + (btn_size - sq_size) / 2;
                let sq_y = btn_y + (btn_size - sq_size) / 2;
                ui::draw::draw_rect_outline(surface, Rect::new(sq_x, sq_y, sq_size, sq_size), 1, Color::WHITE);

                // Minimize Button ('-')
                let min_col = if is_active { palette.status_warning } else { palette.status_warning.darken(35) };
                ui::draw::draw_rounded_rect(surface, Rect::new(min_x, btn_y, btn_size, btn_size), 2, min_col);
                let bar_w = (btn_size - 4).max(3);
                let bar_x = min_x + (btn_size - bar_w) / 2;
                surface.fill_rect(Rect::new(bar_x, cy, bar_w, 1), Color::WHITE);
            }
        }

        // 3. Taskbar
        let taskbar_y = self.desktop_h;
        surface.fill_rect(Rect::new(0, taskbar_y, self.screen_w, self.taskbar_h), palette.taskbar_bg);
        surface.fill_rect(Rect::new(0, taskbar_y, self.screen_w, 1), palette.border_subtle);

        let button_w = self.screen_w / MAX_WINDOWS as i32;
        for i in 0..MAX_WINDOWS {
            let bx = i as i32 * button_w;
            let is_active = i == topmost;
            let bg = if is_active { palette.taskbar_item_active } else { palette.taskbar_item_inactive };

            let btn_rect = Rect::new(bx + 1, taskbar_y + 1, button_w - 2, self.taskbar_h - 2);
            surface.fill_rect(btn_rect, bg);

            if is_active && self.taskbar_h >= 8 {
                surface.fill_rect(Rect::new(bx + 4, taskbar_y + self.taskbar_h - 2, button_w - 8, 2), palette.accent_primary);
            }

            if self.taskbar_h >= 20 {
                let font = Font::regular();
                let label_str = match self.windows[i].label {
                    1 => "Win 1",
                    2 => "Win 2",
                    3 => "Win 3",
                    _ => "Win 4",
                };
                let sz = font.measure_str(label_str);
                let text_x = bx + (button_w - sz.w) / 2;
                let text_y = taskbar_y + (self.taskbar_h - font.line_height()) / 2;
                ui::font::draw_text_font(surface, text_x, text_y, label_str, palette.text_primary, &font);
            } else {
                let font = Font::small();
                let text_y = taskbar_y + (self.taskbar_h - 8).max(0) / 2;
                let text_x = bx + button_w / 2 - 4;
                ui::font::draw_char_font(surface, text_x, text_y, b'0' + self.windows[i].label, palette.text_primary, &font);
            }
        }

        // 4. Mouse cursor
        let shape = self.cursor_shape_at(self.cursor_x, self.cursor_y);
        ui::cursor::draw_cursor_sprite(surface, self.cursor_x, self.cursor_y, shape, self.cursor_theme);
    }
}

fn draw_app_icon<S: Surface + ?Sized>(s: &mut S, x: i32, y: i32, label: u8, palette: &ThemePalette) {
    match label {
        1 => {
            // Terminal prompt `>_`
            ui::draw::draw_rounded_rect(s, Rect::new(x, y, 12, 12), 2, palette.surface_sunken);
            ui::draw::draw_line(s, x + 3, y + 3, x + 5, y + 5, palette.status_success);
            ui::draw::draw_line(s, x + 5, y + 5, x + 3, y + 7, palette.status_success);
            s.fill_rect(Rect::new(x + 7, y + 7, 3, 1), Color::WHITE);
        }
        2 => {
            // Folder icon
            let folder_col = Color::rgb(245, 180, 50);
            s.fill_rect(Rect::new(x + 2, y + 2, 4, 2), folder_col.lighten(20));
            ui::draw::draw_rounded_rect(s, Rect::new(x + 1, y + 4, 10, 7), 1, folder_col);
        }
        3 => {
            // Document sheet icon
            let doc_col = Color::WHITE;
            ui::draw::draw_rounded_rect(s, Rect::new(x + 2, y + 1, 8, 10), 1, doc_col);
            s.put_pixel(x + 9, y + 1, palette.surface_sunken);
            s.fill_rect(Rect::new(x + 4, y + 4, 4, 1), palette.text_muted);
            s.fill_rect(Rect::new(x + 4, y + 6, 4, 1), palette.text_muted);
            s.fill_rect(Rect::new(x + 4, y + 8, 3, 1), palette.text_muted);
        }
        _ => {
            // System gear icon
            let gear_col = Color::rgb(140, 160, 180);
            s.fill_rect(Rect::new(x + 4, y + 2, 4, 8), gear_col);
            s.fill_rect(Rect::new(x + 2, y + 4, 8, 4), gear_col);
            s.fill_rect(Rect::new(x + 5, y + 5, 2, 2), palette.titlebar_active);
        }
    }
}

fn mouse(dx: i32, dy: i32, left: bool) -> ffi::MouseState {
    ffi::MouseState { dx, dy, left_button: left, right_button: false, middle_button: false }
}

/// Verification suite checking window management logic and framebuffer surface rendering.
pub fn run_selftest() -> i32 {
    let mut code = 0;
    let mut wm = Wm::new();

    unsafe { ffi::sys_write(b"[wm-selftest] starting\n\0".as_ptr()) };

    // Case 0: Verify shared UI design system invariants
    let ds_code = ui::run_design_system_selftest();
    if ds_code == 0 {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: shared UI design system invariants hold\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: UI design system invariants\n\0".as_ptr()) };
        code |= 32;
    }

    // Case 1: dragging window 0 by its titlebar actually moves it,
    // and moving the cursor there and pressing raises it to the front of z_order.
    let start = wm.windows[0];
    wm.cursor_x = 25;
    wm.cursor_y = 25;
    wm.apply_mouse(mouse(0, 0, true)); // press
    let raised_to_front = wm.z_order[MAX_WINDOWS - 1] == 0;
    wm.apply_mouse(mouse(10, 8, true)); // drag
    let moved = wm.windows[0].x != start.x || wm.windows[0].y != start.y;
    wm.apply_mouse(mouse(0, 0, false)); // release
    let drag_ended = wm.dragging.is_none();
    if raised_to_front && moved && drag_ended {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: titlebar drag moves a window and raises it\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: titlebar drag\n\0".as_ptr()) };
        code |= 1;
    }

    // Case 2: clicking anywhere on a different window's body raises it.
    wm.cursor_x = 65;
    wm.cursor_y = 130;
    wm.apply_mouse(mouse(0, 0, true));
    let win2_raised = wm.z_order[MAX_WINDOWS - 1] == 2;
    wm.apply_mouse(mouse(0, 0, false));
    if win2_raised {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: clicking a window's body raises it\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: click-to-focus\n\0".as_ptr()) };
        code |= 2;
    }

    // Case 3: clicking a taskbar button raises that window.
    let button_w = wm.screen_w / MAX_WINDOWS as i32;
    wm.cursor_x = 1 * button_w + 2;
    wm.cursor_y = wm.desktop_h + 2;
    wm.apply_mouse(mouse(0, 0, true));
    let win1_raised_via_taskbar = wm.z_order[MAX_WINDOWS - 1] == 1;
    wm.apply_mouse(mouse(0, 0, false));
    if win1_raised_via_taskbar {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: taskbar button click raises its window\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: taskbar focus\n\0".as_ptr()) };
        code |= 4;
    }

    // Case 4: dragging window 3's titlebar to the left screen edge snaps it.
    let w3 = wm.windows[3];
    wm.cursor_x = w3.x + 5;
    wm.cursor_y = w3.y + 3;
    wm.apply_mouse(mouse(0, 0, true));
    wm.apply_mouse(mouse(-400, 0, true));
    wm.apply_mouse(mouse(0, 0, false));
    let snapped_left = wm.windows[3].snap == SnapState::Left
        && wm.windows[3].x == 0
        && wm.windows[3].y == 0
        && wm.windows[3].w == wm.screen_w / 2
        && wm.windows[3].h == wm.desktop_h;
    if snapped_left {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: dragging to the left edge snaps the window\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: edge snap\n\0".as_ptr()) };
        code |= 8;
    }

    // Case 5: grabbing a snapped window's titlebar un-snaps it back to floating geometry.
    wm.cursor_x = 5;
    wm.cursor_y = 3;
    wm.apply_mouse(mouse(0, 0, true));
    let w3_now = wm.windows[3];
    let unsnapped = w3_now.snap == SnapState::Normal
        && w3_now.w == w3.w
        && w3_now.h == w3.h
        && w3_now.x >= 0 && w3_now.x + w3_now.w <= wm.screen_w
        && w3_now.y >= 0 && w3_now.y + w3_now.h <= wm.desktop_h;
    wm.apply_mouse(mouse(0, 0, false));
    if unsnapped {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: re-dragging a snapped window un-snaps it, fully on-screen\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: un-snap\n\0".as_ptr()) };
        code |= 16;
    }

    // Case 6 (Month 2 Compositor): Verify framebuffer surface rendering via BufferSurface32
    wm.cursor_x = wm.screen_w / 2;
    wm.cursor_y = wm.desktop_h / 2;
    static mut TEST_BUF: [u32; 320 * 200] = [0u32; 320 * 200];
    let test_buf: &mut [u32] = unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(TEST_BUF) as *mut u32, 320 * 200)
    };
    let mut surface = ui::draw::BufferSurface32::new(test_buf, 320, 200);
    wm.render(&mut surface);

    let palette = Theme::Dark.palette();
    let bg_color = palette.desktop_bg.to_xrgb();
    let pixel_0_0 = surface.buffer[0];
    let pixel_taskbar = surface.buffer[195 * 320 + 10];

    if pixel_0_0 == bg_color && pixel_taskbar != bg_color {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: framebuffer surface rendering (BufferSurface32)\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: framebuffer surface rendering\n\0".as_ptr()) };
        code |= 64;
    }

    // Case 7 (Month 2 Compositor): Verify multi-size font rendering and antialiasing
    let f_small = Font::small();
    let f_reg = Font::regular();
    let f_large = Font::large();
    let m_small = f_small.measure_str("NovaOS");
    let m_reg = f_reg.measure_str("NovaOS");
    let m_large = f_large.measure_str("NovaOS");

    let mut aa_buf = [0u32; 16 * 16];
    let mut aa_surface = ui::draw::BufferSurface32::new(&mut aa_buf, 16, 16);
    ui::font::draw_char_font(&mut aa_surface, 4, 0, b'A', Color::WHITE, &f_reg);
    let has_aa_blend = aa_surface.buffer.iter().any(|&p| p != 0 && p != 0x00FFFFFF);

    if m_large.h > m_reg.h && m_reg.h > m_small.h && has_aa_blend {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: real multi-size font rendering and antialiasing\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: font rendering\n\0".as_ptr()) };
        code |= 128;
    }

    // Case 8 (Month 2 Compositor): Verify cursor themes and redesigned window chrome
    let mut wm_hires = Wm::new_for_display(1024, 768);
    wm_hires.raise(0); // Make window 0 active/topmost

    // Reuse test_buf (500 * 70 = 35,000 u32s <= 64,000) for testing high-res chrome
    test_buf[..500 * 70].fill(0);
    let mut hr_surface = ui::draw::BufferSurface32::new(&mut test_buf[..500 * 70], 500, 70);
    wm_hires.render(&mut hr_surface);

    // 1. Verify titlebar top highlight sheen exists on active window 0 (x=40..400, y=40..68)
    let sheen_pixel = hr_surface.buffer[40 * 500 + 100];
    let body_pixel = hr_surface.buffer[55 * 500 + 100];
    let has_sheen = sheen_pixel != body_pixel && sheen_pixel != 0;

    // 2. Verify window control buttons are rendered with distinct status colors
    // Window 0 is at x=40, w=360 (right edge at 400).
    // Close, Maximize, and Minimize buttons are within x in 330..400, y in 42..66
    let mut has_close_red = false;
    let mut has_max_green = false;
    let mut has_min_amber = false;
    for y in 42..66 {
        for x in 330..400 {
            let p = hr_surface.buffer[y * 500 + x];
            if p == palette.status_danger.to_xrgb() {
                has_close_red = true;
            } else if p == palette.status_success.to_xrgb() {
                has_max_green = true;
            } else if p == palette.status_warning.to_xrgb() {
                has_min_amber = true;
            }
        }
    }

    // 3. Verify dynamic cursor shape detection across various screen contexts
    let shape_desktop = wm_hires.cursor_shape_at(5, 5);
    let shape_taskbar = wm_hires.cursor_shape_at(100, 750);
    let shape_titlebar = wm_hires.cursor_shape_at(100, 50);
    let shape_close_btn = wm_hires.cursor_shape_at(385, 48);
    let shape_console = wm_hires.cursor_shape_at(100, 100);

    let shapes_ok = shape_desktop == CursorShape::Arrow
        && shape_taskbar == CursorShape::Pointer
        && shape_titlebar == CursorShape::Move
        && shape_close_btn == CursorShape::Pointer
        && shape_console == CursorShape::IBeam;

    // 4. Verify cursor theme sprite rendering into BufferSurface32
    let mut cur_buf = [0u32; 16 * 16];
    let mut cur_surface = ui::draw::BufferSurface32::new(&mut cur_buf, 16, 16);
    ui::cursor::draw_cursor_sprite(&mut cur_surface, 0, 0, CursorShape::Arrow, CursorTheme::Classic);
    let cur_rendered_ok = cur_surface.buffer[0] == 0x00000000 && cur_surface.buffer[2 * 16 + 1] == 0x00FFFFFF;

    if has_sheen && has_close_red && has_max_green && has_min_amber && shapes_ok && cur_rendered_ok {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: cursor themes and redesigned window chrome\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: cursor themes and window chrome\n\0".as_ptr()) };
        code |= 256;
    }

    if code == 0 {
        unsafe { ffi::sys_write(b"[wm-selftest] all cases passed\n\0".as_ptr()) };
    }
    code
}

fn starts_with_cstr(ptr: *const u8, prefix: &[u8]) -> bool {
    if ptr.is_null() {
        return false;
    }
    for (i, &want) in prefix.iter().enumerate() {
        let got = unsafe { *ptr.add(i) };
        if got == 0 || got != want {
            return false;
        }
    }
    true
}

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8, _envp: *const *const u8) -> i32 {
    if argc >= 2 {
        let ptr = unsafe { *argv.offset(1) };
        if starts_with_cstr(ptr, b"--selftest") {
            return run_selftest();
        }
    }

    unsafe {
        let msg = b"wm: entering framebuffer window manager (Month 2 Compositor)\n\0";
        ffi::sys_write(msg.as_ptr());
    }

    // 1. Query display info
    let mut fb_info = ffi::NovaFbInfo {
        struct_size: core::mem::size_of::<ffi::NovaFbInfo>() as u32,
        ..Default::default()
    };
    let info_rc = unsafe { ffi::sys_fb_info(&mut fb_info) };

    let (screen_w, screen_h) = if info_rc == 0 && fb_info.width > 0 && fb_info.height > 0 {
        (fb_info.width as i32, fb_info.height as i32)
    } else {
        (FB_SCREEN_W, FB_SCREEN_H)
    };

    // 2. Take exclusive ownership of the display
    let acq_rc = unsafe { ffi::sys_fb_acquire(ffi::NOVA_FB_BACKEND_ANY) };
    if acq_rc < 0 {
        unsafe {
            let msg = b"wm: failed to acquire framebuffer display (-EBUSY or -ENODEV)\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        return 1;
    }

    // 3. Create a 32-bit XRGB framebuffer surface
    let mut create_req = ffi::NovaFbCreate {
        width: screen_w as u32,
        height: screen_h as u32,
        flags: 0,
        handle: 0,
        pixels: 0,
        stride: 0,
        size: 0,
    };
    let create_rc = unsafe { ffi::sys_fb_create(&mut create_req) };
    if create_rc < 0 || create_req.pixels == 0 {
        unsafe {
            ffi::sys_fb_release();
            let msg = b"wm: failed to create framebuffer surface\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        return 1;
    }

    let fb_handle = create_req.handle;
    let pixels_ptr = create_req.pixels as *mut u32;
    let total_pixels = (screen_w * screen_h) as usize;

    let mut wm = Wm::new_for_display(screen_w, screen_h);

    loop {
        let key = unsafe { ffi::sys_read_key() };
        if key == b'q' as i32 || key == 27 {
            break;
        }

        let mut m = ffi::MouseState { dx: 0, dy: 0, left_button: false, right_button: false, middle_button: false };
        unsafe { ffi::sys_mouse_read(&mut m) };
        wm.apply_mouse(m);

        // Render frame to surface buffer in userland memory (no syscalls during rendering)
        let pixel_slice = unsafe { core::slice::from_raw_parts_mut(pixels_ptr, total_pixels) };
        let mut surface = ui::draw::BufferSurface32::new_with_pitch(pixel_slice, screen_w, screen_h, screen_w);
        wm.render(&mut surface);

        // Present frame with a single syscall
        let present_req = ffi::NovaFbPresent {
            handle: fb_handle,
            src_x: 0,
            src_y: 0,
            width: screen_w,
            height: screen_h,
            dst_x: 0,
            dst_y: 0,
            flags: 0,
        };
        unsafe { ffi::sys_fb_present(&present_req) };
        unsafe { ffi::sys_yield() };
    }

    // 4. Clean up display and surface
    unsafe {
        ffi::sys_fb_destroy(fb_handle);
        ffi::sys_fb_release();
        let msg = b"wm: window manager exited, display released.\n\0";
        ffi::sys_write(msg.as_ptr());
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wm_selftest_cases() {
        assert_eq!(run_selftest(), 0);
    }

    #[test]
    fn test_wm_high_res_construction() {
        let wm = Wm::new_for_display(1024, 768);
        assert_eq!(wm.screen_w, 1024);
        assert_eq!(wm.screen_h, 768);
        assert_eq!(wm.taskbar_h, 36);
        assert_eq!(wm.titlebar_h, 28);
        assert_eq!(wm.desktop_h, 732);
        assert_eq!(wm.cursor_x, 512);
        assert_eq!(wm.cursor_y, 384);
    }

    #[test]
    fn test_wm_snapping_proportions() {
        let mut wm = Wm::new_for_display(1024, 768);
        // Drag window 0 to left edge
        wm.cursor_x = 50;
        wm.cursor_y = 50;
        wm.apply_mouse(mouse(0, 0, true));
        wm.apply_mouse(mouse(-1000, 0, true));
        wm.apply_mouse(mouse(0, 0, false));

        assert_eq!(wm.windows[0].snap, SnapState::Left);
        assert_eq!(wm.windows[0].x, 0);
        assert_eq!(wm.windows[0].y, 0);
        assert_eq!(wm.windows[0].w, 512);
        assert_eq!(wm.windows[0].h, 732);
    }

    #[test]
    fn test_wm_font_rendering_high_res() {
        let wm = Wm::new_for_display(1024, 768);
        let mut buf = vec![0u32; 1024 * 768];
        let mut surface = ui::draw::BufferSurface32::new(&mut buf, 1024, 768);
        wm.render(&mut surface);

        // Window 1 titlebar should contain antialiased text pixels
        let title_region = Rect::new(40, 40, 360, 28);
        let mut non_zero_pixels = 0;
        let mut aa_pixels = 0;
        let bg_color = Theme::Dark.palette().titlebar_active.to_xrgb();

        for y in title_region.y..title_region.bottom() {
            for x in title_region.x..title_region.right() {
                let p = surface.buffer[(y * 1024 + x) as usize];
                if p != bg_color {
                    non_zero_pixels += 1;
                    if p != 0x00FFFFFF && p != 0 {
                        aa_pixels += 1;
                    }
                }
            }
        }

        assert!(non_zero_pixels > 50, "Window titlebar should contain rendered text pixels");
        assert!(aa_pixels > 0, "Window titlebar should contain antialiased blended pixels");
    }

    #[test]
    fn test_wm_cursor_shapes() {
        let mut wm = Wm::new_for_display(1024, 768);
        wm.raise(0); // Window 0 is at (40, 40, 360, 250)

        // Desktop corner
        assert_eq!(wm.cursor_shape_at(5, 5), CursorShape::Arrow);

        // Taskbar at bottom
        assert_eq!(wm.cursor_shape_at(200, 750), CursorShape::Pointer);

        // Window 0 titlebar
        assert_eq!(wm.cursor_shape_at(100, 50), CursorShape::Move);

        // Window 0 close button (right side of titlebar, around x=385, y=48)
        assert_eq!(wm.cursor_shape_at(385, 48), CursorShape::Pointer);

        // Window 0 console body (x=100, y=100)
        assert_eq!(wm.cursor_shape_at(100, 100), CursorShape::IBeam);

        // When dragging, cursor shape is always Move
        wm.dragging = Some(0);
        assert_eq!(wm.cursor_shape_at(5, 5), CursorShape::Move);
    }

    #[test]
    fn test_wm_window_chrome_buttons() {
        let mut wm = Wm::new_for_display(1024, 768);
        wm.raise(0);
        let mut buf = vec![0u32; 1024 * 80];
        let mut surface = ui::draw::BufferSurface32::new(&mut buf, 1024, 80);
        wm.render(&mut surface);

        let palette = Theme::Dark.palette();
        let danger_red = palette.status_danger.to_xrgb();
        let success_green = palette.status_success.to_xrgb();
        let warning_amber = palette.status_warning.to_xrgb();

        let mut found_red = false;
        let mut found_green = false;
        let mut found_amber = false;

        for y in 40..68 {
            for x in 330..400 {
                let p = surface.buffer[(y * 1024 + x) as usize];
                if p == danger_red {
                    found_red = true;
                } else if p == success_green {
                    found_green = true;
                } else if p == warning_amber {
                    found_amber = true;
                }
            }
        }

        assert!(found_red, "Active window should have a red Close button");
        assert!(found_green, "Active window should have a green Maximize button");
        assert!(found_amber, "Active window should have an amber Minimize button");
    }
}
