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
pub const FB_TITLEBAR_H: i32 = TITLEBAR_HEIGHT; // 28px
pub const FB_TASKBAR_H: i32 = TASKBAR_HEIGHT;   // 36px
pub const FB_MENUBAR_H: i32 = MENUBAR_HEIGHT;   // 24px
pub const FB_DESKTOP_H: i32 = FB_SCREEN_H - FB_TASKBAR_H; // 732px

pub const MAX_WINDOWS: usize = 4;
pub const MAX_DOCK_ITEMS: usize = 8;
pub const SNAP_MARGIN: i32 = 4;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SnapState {
    Normal,
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Debug)]
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
    pub is_open: bool,
    pub is_minimized: bool,
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
            is_open: true,
            is_minimized: false,
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
    pub menubar_h: i32,
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
    pub active_menu: Option<usize>,
    pub hovered_menu: Option<usize>,
    pub hovered_item: Option<usize>,
    pub last_menu_event: Option<MenuClickEvent>,
    pub dock_apps: [DockApp; MAX_DOCK_ITEMS],
    pub dock_count: usize,
    pub hovered_dock_item: Option<usize>,
    pub last_dock_event: Option<DockEvent>,
}

impl Wm {
    /// Constructs default WM geometry matching the 320x200 layout,
    /// ensuring deterministic compatibility with automated self-tests.
    pub fn new() -> Self {
        let default_apps = [
            DEFAULT_PINNED_APPS[0],
            DEFAULT_PINNED_APPS[1],
            DEFAULT_PINNED_APPS[2],
            DEFAULT_PINNED_APPS[3],
            DEFAULT_PINNED_APPS[4],
            DockApp::new(0, "", "", false),
            DockApp::new(0, "", "", false),
            DockApp::new(0, "", "", false),
        ];
        Wm {
            screen_w: DEFAULT_SCREEN_W,
            screen_h: DEFAULT_SCREEN_H,
            titlebar_h: DEFAULT_TITLEBAR_H,
            taskbar_h: DEFAULT_TASKBAR_H,
            menubar_h: 0,
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
            active_menu: None,
            hovered_menu: None,
            hovered_item: None,
            last_menu_event: None,
            dock_apps: default_apps,
            dock_count: 4,
            hovered_dock_item: None,
            last_dock_event: None,
        }
    }

    /// Constructs WM layout scaled for high-resolution 32-bit linear framebuffers.
    pub fn new_for_display(width: i32, height: i32) -> Self {
        let taskbar_h = if height >= 600 { FB_TASKBAR_H } else { DEFAULT_TASKBAR_H };
        let titlebar_h = if height >= 600 { FB_TITLEBAR_H } else { DEFAULT_TITLEBAR_H };
        let menubar_h = if height >= 600 { FB_MENUBAR_H } else { 0 };
        let desktop_h = height - taskbar_h;

        let palette = Theme::Dark.palette();

        let (win_w, win_h) = if width >= 800 { (360, 250) } else { (160, 110) };
        let w0 = Window::new(40, 40, win_w, win_h, palette.surface_raised, palette.titlebar_active, 1);
        let w1 = Window::new(440, 60, win_w, win_h, palette.surface_raised, palette.titlebar_inactive, 2);
        let w2 = Window::new(120, 330, win_w, win_h, palette.surface_raised, palette.titlebar_inactive, 3);
        let w3 = Window::new(520, 350, win_w, win_h, palette.surface_raised, palette.titlebar_inactive, 4);

        let default_apps = [
            DEFAULT_PINNED_APPS[0],
            DEFAULT_PINNED_APPS[1],
            DEFAULT_PINNED_APPS[2],
            DEFAULT_PINNED_APPS[3],
            DEFAULT_PINNED_APPS[4],
            DockApp::new(0, "", "", false),
            DockApp::new(0, "", "", false),
            DockApp::new(0, "", "", false),
        ];

        let dock_count = if height >= 600 { 5 } else { 4 };

        Wm {
            screen_w: width,
            screen_h: height,
            titlebar_h,
            taskbar_h,
            menubar_h,
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
            active_menu: None,
            hovered_menu: None,
            hovered_item: None,
            last_menu_event: None,
            dock_apps: default_apps,
            dock_count,
            hovered_dock_item: None,
            last_dock_event: None,
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
            if !w.is_open || w.is_minimized {
                continue;
            }
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
            if !w.is_open || w.is_minimized {
                continue;
            }
            if point_in_rect(px, py, w.x, w.y, w.w, w.h) {
                return Some(idx);
            }
        }
        None
    }

    /// Returns the topmost window that is open and not minimized.
    pub fn active_window(&self) -> Option<usize> {
        for i in (0..MAX_WINDOWS).rev() {
            let idx = self.z_order[i];
            let w = &self.windows[idx];
            if w.is_open && !w.is_minimized {
                return Some(idx);
            }
        }
        None
    }

    /// Hit-tests pointer coordinates against dock / taskbar items.
    pub fn dock_item_at(&self, px: i32, py: i32) -> Option<usize> {
        if py < self.desktop_h || py >= self.screen_h || px < 0 || px >= self.screen_w {
            return None;
        }

        if self.taskbar_h >= 24 {
            let taskbar_rect = Rect::new(0, self.desktop_h, self.screen_w, self.taskbar_h);
            let start_w = 36;
            let start_offset_x = start_w + 12;
            let max_item_w = if self.screen_w >= 800 { 120 } else { 80 };

            for i in 0..self.dock_count {
                if let Some(rect) = ui::dock::measure_dock_item(i, self.dock_count, taskbar_rect, start_offset_x, max_item_w) {
                    if point_in_rect(px, py, rect.x, rect.y, rect.w, rect.h) {
                        return Some(i);
                    }
                }
            }
            None
        } else {
            let count = self.dock_count.max(1);
            let button_w = self.screen_w / count as i32;
            if button_w <= 0 {
                return None;
            }
            let idx = (px / button_w) as usize;
            if idx < count {
                Some(idx)
            } else {
                None
            }
        }
    }

    pub fn taskbar_button_at(&self, px: i32, py: i32) -> Option<usize> {
        self.dock_item_at(px, py)
    }

    /// Returns the MenuBar for the window at `win_index`.
    pub fn app_menubar(&self, win_index: usize) -> ui::menu::MenuBar {
        if win_index < MAX_WINDOWS {
            ui::menu::menubar_for_label(self.windows[win_index].label)
        } else {
            ui::menu::menubar_for_label(1)
        }
    }

    /// Returns the MenuBar for the currently focused (active) window.
    pub fn current_menubar(&self) -> ui::menu::MenuBar {
        if let Some(topmost) = self.active_window() {
            self.app_menubar(topmost)
        } else {
            ui::menu::SYSTEM_MENUBAR
        }
    }

    /// Hit-tests a point against the top menu bar items.
    pub fn menubar_item_at(&self, px: i32, py: i32) -> Option<usize> {
        if self.menubar_h <= 0 || py < 0 || py >= self.menubar_h {
            return None;
        }
        let menubar = self.current_menubar();
        let font = if self.menubar_h >= 20 { Font::regular() } else { Font::small() };
        let star_size = if self.menubar_h >= 20 { 12 } else { 8 };
        let star_x = 8;
        let app_sz = font.measure_str(menubar.app_name);
        let app_x = star_x + star_size + 8;
        let start_x = app_x + app_sz.w + 18;

        for (i, _) in menubar.menus.iter().enumerate() {
            if let Some(r) = ui::menu::measure_menubar_item(&menubar, i, start_x, 0, self.menubar_h, &font) {
                if point_in_rect(px, py, r.x, r.y, r.w, r.h) {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Returns the screen bounds of the currently open dropdown menu.
    pub fn active_dropdown_rect(&self) -> Option<Rect> {
        if self.menubar_h <= 0 {
            return None;
        }
        let menu_idx = self.active_menu?;
        let menubar = self.current_menubar();
        if menu_idx >= menubar.menus.len() {
            return None;
        }
        let font = if self.menubar_h >= 20 { Font::regular() } else { Font::small() };
        let star_size = if self.menubar_h >= 20 { 12 } else { 8 };
        let star_x = 8;
        let app_sz = font.measure_str(menubar.app_name);
        let app_x = star_x + star_size + 8;
        let start_x = app_x + app_sz.w + 18;

        let item_rect = ui::menu::measure_menubar_item(&menubar, menu_idx, start_x, 0, self.menubar_h, &font)?;
        let menu = &menubar.menus[menu_idx];
        Some(ui::menu::measure_dropdown_rect(item_rect.x, self.menubar_h, menu, self.screen_w, &font))
    }

    /// Hit-tests a point against the items inside the active dropdown menu.
    pub fn dropdown_item_at(&self, px: i32, py: i32) -> Option<usize> {
        let rect = self.active_dropdown_rect()?;
        if !point_in_rect(px, py, rect.x, rect.y, rect.w, rect.h) {
            return None;
        }
        let menu_idx = self.active_menu?;
        let menubar = self.current_menubar();
        let menu = menubar.menus.get(menu_idx)?;
        let font = if rect.h > 100 { Font::regular() } else { Font::small() };
        let item_h = if font.line_height() > 10 { 22 } else { 14 };

        let mut cur_y = rect.y + 4;
        for (i, item) in menu.items.iter().enumerate() {
            let h = if item.is_separator { 6 } else { item_h };
            if py >= cur_y && py < cur_y + h {
                if !item.is_separator && item.is_enabled {
                    return Some(i);
                } else {
                    return None;
                }
            }
            cur_y += h;
        }
        None
    }

    /// Drains and returns the most recent menu click event, if any.
    pub fn take_menu_event(&mut self) -> Option<MenuClickEvent> {
        self.last_menu_event.take()
    }

    /// Computes the runtime state of all items currently displayed in the dock/taskbar.
    pub fn dock_items(&self) -> [Option<DockItem>; MAX_DOCK_ITEMS] {
        let mut items = [None; MAX_DOCK_ITEMS];
        let active_win = self.active_window();

        for i in 0..self.dock_count {
            let app = self.dock_apps[i];
            let mut matching_win = None;
            for (w_idx, w) in self.windows.iter().enumerate() {
                if w.is_open && w.label == app.app_id {
                    matching_win = Some((w_idx, w));
                    break;
                }
            }

            if let Some((w_idx, w)) = matching_win {
                let is_active = active_win == Some(w_idx);
                items[i] = Some(DockItem::new(
                    app,
                    true,
                    is_active,
                    w.is_minimized,
                    Some(w_idx),
                ));
            } else {
                items[i] = Some(DockItem::new(
                    app,
                    false,
                    false,
                    false,
                    None,
                ));
            }
        }
        items
    }

    /// Handles a click on the dock / taskbar item at index `idx`.
    pub fn on_dock_item_click(&mut self, idx: usize) {
        if idx >= self.dock_count {
            return;
        }
        let app = self.dock_apps[idx];
        let items = self.dock_items();
        let item = match items[idx] {
            Some(it) => it,
            None => return,
        };

        if item.is_running {
            if let Some(win_idx) = item.window_id {
                let is_active = self.active_window() == Some(win_idx);
                if is_active {
                    // Clicking active focused window minimizes it to taskbar
                    self.minimize_window(win_idx);
                } else if self.windows[win_idx].is_minimized {
                    // Clicking minimized window restores and focuses it
                    self.restore_window(win_idx);
                } else {
                    // Clicking inactive running window raises it to front
                    self.raise(win_idx);
                    self.last_dock_event = Some(DockEvent::Switch { window_id: win_idx });
                }
            }
        } else {
            // App is pinned but NOT currently running: Launch-on-click!
            self.last_dock_event = Some(DockEvent::Launch {
                app_id: app.app_id,
                exec_path: app.exec_path,
            });

            // Re-open window if matching slot exists
            for (w_idx, w) in self.windows.iter_mut().enumerate() {
                if w.label == app.app_id {
                    w.is_open = true;
                    w.is_minimized = false;
                    self.raise(w_idx);
                    return;
                }
            }

            // Or allocate first closed slot
            for (w_idx, w) in self.windows.iter_mut().enumerate() {
                if !w.is_open {
                    w.label = app.app_id;
                    w.is_open = true;
                    w.is_minimized = false;
                    self.raise(w_idx);
                    return;
                }
            }
        }
    }

    /// Minimizes `win_idx` to the dock / taskbar.
    pub fn minimize_window(&mut self, win_idx: usize) {
        if win_idx < MAX_WINDOWS {
            self.windows[win_idx].is_minimized = true;
            self.last_dock_event = Some(DockEvent::Minimize { window_id: win_idx });
        }
    }

    /// Restores a minimized window and brings it to front.
    pub fn restore_window(&mut self, win_idx: usize) {
        if win_idx < MAX_WINDOWS {
            self.windows[win_idx].is_minimized = false;
            self.windows[win_idx].is_open = true;
            self.raise(win_idx);
            self.last_dock_event = Some(DockEvent::Restore { window_id: win_idx });
        }
    }

    /// Closes window `win_idx` while preserving its pinned dock status.
    pub fn close_window(&mut self, win_idx: usize) {
        if win_idx < MAX_WINDOWS {
            self.windows[win_idx].is_open = false;
            self.windows[win_idx].is_minimized = false;
        }
    }

    /// Pins `app` to the dock / taskbar.
    pub fn pin_app(&mut self, app: DockApp) -> bool {
        for i in 0..self.dock_count {
            if self.dock_apps[i].app_id == app.app_id {
                self.dock_apps[i].is_pinned = true;
                self.last_dock_event = Some(DockEvent::Pin { app_id: app.app_id });
                return true;
            }
        }
        if self.dock_count < MAX_DOCK_ITEMS {
            self.dock_apps[self.dock_count] = app;
            self.dock_count += 1;
            self.last_dock_event = Some(DockEvent::Pin { app_id: app.app_id });
            true
        } else {
            false
        }
    }

    /// Unpins `app_id` from the dock.
    pub fn unpin_app(&mut self, app_id: u8) -> bool {
        for i in 0..self.dock_count {
            if self.dock_apps[i].app_id == app_id {
                let items = self.dock_items();
                let is_running = items[i].map_or(false, |it| it.is_running);
                if is_running {
                    self.dock_apps[i].is_pinned = false;
                } else {
                    for j in i..self.dock_count - 1 {
                        self.dock_apps[j] = self.dock_apps[j + 1];
                    }
                    self.dock_count -= 1;
                }
                self.last_dock_event = Some(DockEvent::Unpin { app_id });
                return true;
            }
        }
        false
    }

    /// Checks if `app_id` is pinned in the dock.
    pub fn is_app_pinned(&self, app_id: u8) -> bool {
        for i in 0..self.dock_count {
            if self.dock_apps[i].app_id == app_id && self.dock_apps[i].is_pinned {
                return true;
            }
        }
        false
    }

    /// Drains and returns the most recent dock event, if any.
    pub fn take_dock_event(&mut self) -> Option<DockEvent> {
        self.last_dock_event.take()
    }

    /// Evaluates dynamic cursor shape based on current pointer coordinates.
    pub fn cursor_shape_at(&self, cx: i32, cy: i32) -> CursorShape {
        if self.dragging.is_some() {
            return CursorShape::Move;
        }
        // Active dropdown menu inspection
        if let Some(rect) = self.active_dropdown_rect() {
            if point_in_rect(cx, cy, rect.x, rect.y, rect.w, rect.h) {
                if self.dropdown_item_at(cx, cy).is_some() {
                    return CursorShape::Pointer;
                }
                return CursorShape::Arrow;
            }
        }
        // Top menubar inspection
        if self.menubar_h > 0 && cy < self.menubar_h {
            if self.menubar_item_at(cx, cy).is_some() {
                return CursorShape::Pointer;
            }
            return CursorShape::Arrow;
        }
        if self.taskbar_button_at(cx, cy).is_some() {
            return CursorShape::Pointer;
        }
        for i in (0..MAX_WINDOWS).rev() {
            let win_idx = self.z_order[i];
            let w = &self.windows[win_idx];
            if !w.is_open || w.is_minimized {
                continue;
            }
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
        // 1. If dropdown menu is open:
        if self.active_menu.is_some() {
            if let Some(item_idx) = self.dropdown_item_at(px, py) {
                let menu_idx = self.active_menu.unwrap();
                let topmost = self.z_order[MAX_WINDOWS - 1];
                let menubar = self.current_menubar();
                let action_id = menubar.menus[menu_idx].items[item_idx].action_id;
                self.last_menu_event = Some(MenuClickEvent {
                    window_id: topmost,
                    menu_index: menu_idx,
                    item_index: item_idx,
                    action_id,
                });
                self.active_menu = None;
                return;
            }
            // Clicking inside dropdown panel on separator/padding
            if let Some(drop_rect) = self.active_dropdown_rect() {
                if point_in_rect(px, py, drop_rect.x, drop_rect.y, drop_rect.w, drop_rect.h) {
                    return;
                }
            }
            // Clicking a top menu item toggles or switches
            if let Some(menu_idx) = self.menubar_item_at(px, py) {
                if self.active_menu == Some(menu_idx) {
                    self.active_menu = None;
                } else {
                    self.active_menu = Some(menu_idx);
                }
                return;
            }
            // Clicking outside dismisses dropdown
            self.active_menu = None;
        }

        // 2. Top menu bar clicks
        if self.menubar_h > 0 && py < self.menubar_h {
            if let Some(menu_idx) = self.menubar_item_at(px, py) {
                self.active_menu = Some(menu_idx);
            }
            return;
        }

        // 3. Taskbar button clicks
        if let Some(idx) = self.taskbar_button_at(px, py) {
            self.on_dock_item_click(idx);
            return;
        }

        // 4. Window titlebar clicks (raise and start drag)
        if let Some(idx) = self.titlebar_at(px, py) {
            let w = &mut self.windows[idx];
            if self.titlebar_h >= 14 && w.w >= 80 {
                let btn_size = if self.titlebar_h >= 20 { 12 } else { 8 };
                let btn_y = w.y + (self.titlebar_h - btn_size) / 2;
                let close_x = w.x + w.w - btn_size - 8;
                let min_x = close_x - (btn_size + 6) * 2;

                if py >= btn_y && py < btn_y + btn_size {
                    if px >= close_x && px < close_x + btn_size {
                        self.close_window(idx);
                        return;
                    } else if px >= min_x && px < min_x + btn_size {
                        self.minimize_window(idx);
                        return;
                    }
                }
            }

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

        // 5. Window body clicks (raise)
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

        self.hovered_menu = self.menubar_item_at(self.cursor_x, self.cursor_y);
        self.hovered_item = self.dropdown_item_at(self.cursor_x, self.cursor_y);
        self.hovered_dock_item = self.dock_item_at(self.cursor_x, self.cursor_y);

        // If dropdown is open, hovering over another top menu switches to that dropdown
        if self.active_menu.is_some() {
            if let Some(m_idx) = self.hovered_menu {
                self.active_menu = Some(m_idx);
            }
        }

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
        let active_win = self.active_window();
        for i in 0..MAX_WINDOWS {
            let win_idx = self.z_order[i];
            let w = &self.windows[win_idx];
            if !w.is_open || w.is_minimized {
                continue;
            }
            let is_active = Some(win_idx) == active_win;

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

                // Render client application view on high-res
                if w.label == 1 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_term_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &palette);
                } else if w.label == 2 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_files_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &palette);
                } else if w.label == 3 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_editor_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &palette);
                } else if w.label == 4 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_store_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &palette);
                } else if w.label == 5 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_settings_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &palette);
                }
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
                    4 => "Software Store - Packages",
                    5 => "Settings - Control Panel",
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

        // 3. Taskbar / Dock
        if self.taskbar_h >= 24 {
            let dock_items = self.dock_items();
            let mut valid_items = [DockItem::new(self.dock_apps[0], false, false, false, None); MAX_DOCK_ITEMS];
            let mut valid_count = 0;
            for i in 0..self.dock_count {
                if let Some(it) = dock_items[i] {
                    valid_items[valid_count] = it;
                    valid_count += 1;
                }
            }
            let taskbar_rect = Rect::new(0, self.desktop_h, self.screen_w, self.taskbar_h);
            ui::dock::draw_dock_surface(
                surface,
                taskbar_rect,
                &valid_items[..valid_count],
                self.hovered_dock_item,
                "12:45 PM",
                &palette,
            );
        } else {
            let taskbar_y = self.desktop_h;
            surface.fill_rect(Rect::new(0, taskbar_y, self.screen_w, self.taskbar_h), palette.taskbar_bg);
            surface.fill_rect(Rect::new(0, taskbar_y, self.screen_w, 1), palette.border_subtle);

            let count = self.dock_count.max(1);
            let button_w = self.screen_w / count as i32;
            let active_win = self.active_window();

            for i in 0..count {
                let bx = i as i32 * button_w;
                let is_active = active_win == Some(i);
                let is_minimized = i < MAX_WINDOWS && self.windows[i].is_minimized;
                let bg = if is_active {
                    palette.taskbar_item_active
                } else if is_minimized {
                    palette.surface_sunken
                } else {
                    palette.taskbar_item_inactive
                };

                let btn_rect = Rect::new(bx + 1, taskbar_y + 1, button_w - 2, self.taskbar_h - 2);
                surface.fill_rect(btn_rect, bg);

                if is_active && self.taskbar_h >= 8 {
                    surface.fill_rect(Rect::new(bx + 4, taskbar_y + self.taskbar_h - 2, button_w - 8, 2), palette.accent_primary);
                }

                let font = Font::small();
                let text_y = taskbar_y + (self.taskbar_h - 8).max(0) / 2;
                let text_x = bx + button_w / 2 - 4;
                let lbl_char = if i < MAX_WINDOWS { b'0' + self.windows[i].label } else { b'0' + (i as u8 + 1) };
                ui::font::draw_char_font(surface, text_x, text_y, lbl_char, palette.text_primary, &font);
            }
        }

        // 4. Persistent Top Menu Bar (always-on-top compositor layer)
        if self.menubar_h > 0 {
            let menubar = self.current_menubar();
            let menubar_rect = Rect::new(0, 0, self.screen_w, self.menubar_h);
            ui::menu::draw_menubar_surface(
                surface,
                menubar_rect,
                &menubar,
                self.active_menu,
                self.hovered_menu,
                "12:45 PM",
                &palette,
            );
        }

        // 5. Active Dropdown Menu Overlay
        if let Some(drop_rect) = self.active_dropdown_rect() {
            if let Some(idx) = self.active_menu {
                let menubar = self.current_menubar();
                if idx < menubar.menus.len() {
                    let menu = &menubar.menus[idx];
                    ui::menu::draw_dropdown_surface(
                        surface,
                        drop_rect,
                        menu,
                        self.hovered_item,
                        &palette,
                    );
                }
            }
        }

        // 6. Mouse cursor
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
        4 => {
            // Store shopping bag / package icon
            ui::dock::draw_dock_app_icon(s, x, y, ui::dock::APP_ID_STORE, palette);
        }
        5 => {
            // Settings sliders icon
            ui::dock::draw_dock_app_icon(s, x, y, 5, palette);
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

/// Renders a graphical file manager view inside the client area of Window 1.
fn draw_files_client_body<S: Surface + ?Sized>(s: &mut S, client: Rect, palette: &ThemePalette) {
    let f_small = Font::small();

    // 1. Client toolbar
    let tb_h = 24;
    let tb_rect = Rect::new(client.x, client.y, client.w, tb_h);
    s.fill_rect(tb_rect, palette.surface_raised);
    s.fill_rect(Rect::new(client.x, tb_rect.bottom() - 1, client.w, 1), palette.border_subtle);

    // Path pill
    let path_rect = Rect::new(client.x + 6, client.y + 3, 64, 18);
    ui::draw::draw_rounded_rect(s, path_rect, 2, palette.surface_sunken);
    ui::draw::draw_rect_outline(s, path_rect, 1, palette.border_subtle);
    ui::font::draw_text_font(s, path_rect.x + 6, path_rect.y + 2, "/root", palette.text_primary, &f_small);

    // Mini buttons: Open, Move, Delete
    let btns: &[(&str, Color)] = &[
        ("Open", palette.accent_primary),
        ("Move", palette.surface_raised.lighten(30)),
        ("Delete", palette.status_danger),
    ];
    let mut bx = path_rect.right() + 8;
    for &(label, col) in btns {
        if bx + 42 > client.right() - 4 {
            break;
        }
        let b_rect = Rect::new(bx, client.y + 3, 40, 18);
        ui::draw::draw_rounded_rect(s, b_rect, 2, col);
        let sz = f_small.measure_str(label);
        let tx = b_rect.x + (b_rect.w - sz.w) / 2;
        let ty = b_rect.y + (b_rect.h - f_small.line_height()) / 2;
        ui::font::draw_text_font(s, tx, ty, label, Color::WHITE, &f_small);
        bx += 44;
    }

    // 2. Table Column Headers
    let hdr_y = tb_rect.bottom();
    let hdr_h = 18;
    let hdr_rect = Rect::new(client.x, hdr_y, client.w, hdr_h);
    s.fill_rect(hdr_rect, palette.surface_raised.darken(10));
    s.fill_rect(Rect::new(client.x, hdr_rect.bottom() - 1, client.w, 1), palette.border_subtle);

    ui::font::draw_text_font(s, client.x + 24, hdr_y + 2, "Name", palette.text_muted, &f_small);
    if client.w >= 200 {
        ui::font::draw_text_font(s, client.x + 130, hdr_y + 2, "Size", palette.text_muted, &f_small);
    }
    if client.w >= 280 {
        ui::font::draw_text_font(s, client.x + 190, hdr_y + 2, "Type", palette.text_muted, &f_small);
    }

    // 3. File Rows
    let rows: &[(&str, &str, &str, Color)] = &[
        ("HELLO.TXT",     "68 B",   "Document", Color::WHITE),
        ("SHELL.ELF",     "4.0 KB", "Executable", palette.status_success),
        ("SYSTEM.CFG",    "512 B",  "Config", Color::rgb(140, 190, 240)),
        ("FIREWALL.CFG",  "1.2 KB", "Config", Color::rgb(140, 190, 240)),
    ];

    let mut ry = hdr_rect.bottom() + 1;
    let row_h = 20;
    for (idx, &(name, size, kind, icon_col)) in rows.iter().enumerate() {
        if ry + row_h > client.bottom() - 18 {
            break;
        }
        // Selection on row 0
        if idx == 0 {
            s.fill_rect(Rect::new(client.x + 1, ry, client.w - 2, row_h), palette.accent_primary.darken(40));
        }

        // Mini file icon
        ui::draw::draw_rounded_rect(s, Rect::new(client.x + 8, ry + 4, 8, 10), 1, icon_col);

        // Name
        let name_col = if idx == 0 { Color::WHITE } else { palette.text_primary };
        ui::font::draw_text_font(s, client.x + 24, ry + 3, name, name_col, &f_small);

        // Size
        if client.w >= 200 {
            ui::font::draw_text_font(s, client.x + 130, ry + 3, size, palette.text_secondary, &f_small);
        }

        // Type
        if client.w >= 280 {
            ui::font::draw_text_font(s, client.x + 190, ry + 3, kind, palette.text_secondary, &f_small);
        }

        ry += row_h;
    }

    // 4. Status Bar at bottom
    if client.h >= 100 {
        let sb_h = 18;
        let sb_rect = Rect::new(client.x, client.bottom() - sb_h, client.w, sb_h);
        s.fill_rect(sb_rect, palette.surface_raised);
        s.fill_rect(Rect::new(client.x, sb_rect.y, client.w, 1), palette.border_subtle);
        ui::font::draw_text_font(s, client.x + 8, sb_rect.y + 2, "4 files | 5.8 KB", palette.text_muted, &f_small);
    }
}

/// Renders a graphical settings control panel view inside the client area of Window 4 / Settings.
fn draw_settings_client_body<S: Surface + ?Sized>(s: &mut S, client: Rect, palette: &ThemePalette) {
    let f_small = Font::small();
    let font = Font::regular();

    // 1. Sidebar with category badge tabs (w = 84)
    let sb_w = 84;
    let sb_rect = Rect::new(client.x, client.y, sb_w, client.h);
    s.fill_rect(sb_rect, palette.surface_raised);
    s.fill_rect(Rect::new(sb_rect.right() - 1, sb_rect.y, 1, sb_rect.h), palette.border_subtle);

    let tabs: &[(&str, &str)] = &[
        ("SYS",  "System"),
        ("USR",  "Users"),
        ("SVC",  "Services"),
        ("FW",   "Firewall"),
        ("DISP", "Display"),
    ];

    let mut ty = client.y + 6;
    let tab_h = 22;
    for (i, &(badge, name)) in tabs.iter().enumerate() {
        if ty + tab_h > client.bottom() - 18 {
            break;
        }
        let t_rect = Rect::new(client.x + 4, ty, sb_w - 8, tab_h);
        if i == 0 {
            ui::draw::draw_rounded_rect(s, t_rect, 2, palette.accent_primary.darken(35));
            ui::draw::draw_rect_outline(s, t_rect, 1, palette.border_focused);
            s.fill_rect(Rect::new(t_rect.x + 1, t_rect.y + 3, 2, t_rect.h - 6), palette.accent_primary);
        }

        let b_sz = f_small.measure_str(badge);
        let b_rect = Rect::new(t_rect.x + 4, t_rect.y + 3, b_sz.w + 4, t_rect.h - 6);
        let b_bg = if i == 0 { palette.accent_primary } else { palette.surface_sunken };
        ui::draw::draw_rounded_rect(s, b_rect, 2, b_bg);
        ui::font::draw_text_font(s, b_rect.x + 2, b_rect.y + 2, badge, Color::WHITE, &f_small);

        let col = if i == 0 { Color::WHITE } else { palette.text_secondary };
        ui::font::draw_text_font(s, b_rect.right() + 4, t_rect.y + 4, name, col, &f_small);

        ty += tab_h + 2;
    }

    // 2. Top Header inside content area
    let cx = client.x + sb_w;
    let cw = client.w - sb_w;
    let hdr_h = 24;
    let hdr_rect = Rect::new(cx, client.y, cw, hdr_h);
    s.fill_rect(hdr_rect, palette.surface_raised);
    s.fill_rect(Rect::new(cx, hdr_rect.bottom() - 1, cw, 1), palette.border_subtle);

    ui::font::draw_text_font(s, cx + 10, client.y + 4, "System & Identity", palette.text_primary, &font);

    // Save button
    let save_rect = Rect::new(client.right() - 48, client.y + 3, 42, 18);
    ui::draw::draw_rounded_rect(s, save_rect, 2, palette.accent_primary);
    let s_sz = f_small.measure_str("Save");
    ui::font::draw_text_font(s, save_rect.x + (save_rect.w - s_sz.w)/2, save_rect.y + 2, "Save", Color::WHITE, &f_small);

    // 3. Settings Fields
    let fx = cx + 12;
    let mut fy = client.y + hdr_h + 8;

    ui::font::draw_text_font(s, fx, fy, "Hostname:", palette.text_muted, &f_small);
    let h_card = Rect::new(fx + 64, fy - 2, 80, 18);
    ui::draw::draw_rounded_rect(s, h_card, 2, palette.surface_sunken);
    ui::draw::draw_rect_outline(s, h_card, 1, palette.border_subtle);
    ui::font::draw_text_font(s, h_card.x + 6, h_card.y + 2, "novaos", palette.text_primary, &f_small);

    fy += 24;
    ui::font::draw_text_font(s, fx, fy, "Init (PID 1):", palette.text_muted, &f_small);
    let i_card = Rect::new(fx + 64, fy - 2, 80, 18);
    ui::draw::draw_rounded_rect(s, i_card, 2, palette.accent_primary.darken(35));
    ui::draw::draw_rect_outline(s, i_card, 1, palette.accent_primary);
    ui::font::draw_text_font(s, i_card.x + 6, i_card.y + 2, "SHELL.ELF", palette.accent_primary, &f_small);

    fy += 24;
    ui::font::draw_text_font(s, fx, fy, "Config File:", palette.text_muted, &f_small);
    ui::font::draw_text_font(s, fx + 64, fy, "SYSTEM.CFG (81 B)", palette.text_secondary, &f_small);

    // 4. Status Bar at bottom
    if client.h >= 100 {
        let sb_h = 18;
        let sb_rect = Rect::new(cx, client.bottom() - sb_h, cw, sb_h);
        s.fill_rect(sb_rect, palette.surface_raised);
        s.fill_rect(Rect::new(cx, sb_rect.y, cw, 1), palette.border_subtle);
        ui::font::draw_text_font(s, cx + 8, sb_rect.y + 2, "Ready | Synchronized", palette.text_muted, &f_small);
    }
}

/// Renders the in-window terminal emulator client view inside Window 0 / Console Shell.
fn draw_term_client_body<S: Surface + ?Sized>(s: &mut S, client: Rect, _palette: &ThemePalette) {
    let font_mono = Font::mono();
    let font_small = Font::small();

    // 1. Dark terminal canvas
    let bg_color = Color::rgb(13, 17, 23); // #0D1117
    s.fill_rect(client, bg_color);

    // 2. Terminal header / status line
    let hdr_h = 18;
    let hdr_rect = Rect::new(client.x, client.y, client.w, hdr_h);
    s.fill_rect(hdr_rect, Color::rgb(22, 27, 34)); // #161B22
    s.fill_rect(Rect::new(client.x, client.y + hdr_h - 1, client.w, 1), Color::rgb(48, 54, 61));

    ui::font::draw_text_font(s, client.x + 8, client.y + 3, "sh - 80x24 (xterm-256color) | pid 14", Color::rgb(139, 148, 158), &font_small);
    let right_status = "UTF-8";
    let st_sz = font_small.measure_str(right_status);
    ui::font::draw_text_font(s, client.right() - st_sz.w - 12, client.y + 3, right_status, Color::rgb(88, 166, 255), &font_small);

    // 3. Terminal text lines
    let mut y = client.y + hdr_h + 6;
    let line_h = 16;
    let pad_x = client.x + 8;

    // Line 1: Shell welcome
    if y + line_h < client.bottom() - 10 {
        ui::font::draw_text_font(s, pad_x, y, "NovaOS Terminal (Month 3 Desktop Apps)", Color::rgb(88, 166, 255), &font_mono);
        y += line_h;
    }

    // Line 2: uname -a output
    if y + line_h < client.bottom() - 10 {
        ui::font::draw_text_font(s, pad_x, y, "$ uname -a", Color::rgb(139, 148, 158), &font_mono);
        y += line_h;
    }
    if y + line_h < client.bottom() - 10 {
        ui::font::draw_text_font(s, pad_x, y, "NovaOS 1.0.0-compositor i686 pc-novaos", Color::rgb(63, 185, 80), &font_mono);
        y += line_h;
    }

    // Line 3: ls output
    if y + line_h < client.bottom() - 10 {
        ui::font::draw_text_font(s, pad_x, y, "$ ls /bin", Color::rgb(139, 148, 158), &font_mono);
        y += line_h;
    }
    if y + line_h < client.bottom() - 10 {
        ui::font::draw_text_font(s, pad_x, y, "FILES.ELF   SETTINGS.ELF   TERM.ELF", Color::rgb(240, 246, 252), &font_mono);
        y += line_h;
    }

    // Line 4: Active prompt and cursor
    if y + line_h < client.bottom() - 10 {
        let user_str = "admin@novaos";
        let u_sz = font_mono.measure_str(user_str);
        ui::font::draw_text_font(s, pad_x, y, user_str, Color::rgb(63, 185, 80), &font_mono);

        let colon_x = pad_x + u_sz.w;
        ui::font::draw_text_font(s, colon_x, y, ":", Color::rgb(139, 148, 158), &font_mono);

        let path_str = "~";
        let p_sz = font_mono.measure_str(path_str);
        ui::font::draw_text_font(s, colon_x + 8, y, path_str, Color::rgb(88, 166, 255), &font_mono);

        let dollar_x = colon_x + 8 + p_sz.w;
        ui::font::draw_text_font(s, dollar_x, y, "$ ", Color::rgb(240, 246, 252), &font_mono);

        let cmd_str = "help";
        let c_sz = font_mono.measure_str(cmd_str);
        ui::font::draw_text_font(s, dollar_x + 16, y, cmd_str, Color::rgb(240, 246, 252), &font_mono);

        // Blinking / solid block cursor
        let cursor_x = dollar_x + 16 + c_sz.w + 2;
        s.fill_rect(Rect::new(cursor_x, y + 2, 8, line_h - 4), Color::rgb(88, 166, 255));
    }

    // 4. Subtle vertical scrollbar on right
    let bar_x = client.right() - 6;
    let bar_h = client.h - hdr_h - 4;
    s.fill_rect(Rect::new(bar_x, client.y + hdr_h + 2, 4, bar_h), Color::rgba(255, 255, 255, 12));
    s.fill_rect(Rect::new(bar_x, client.y + hdr_h + 10, 4, 30.min(bar_h)), Color::rgba(255, 255, 255, 50));
}

/// Renders the graphical text editor client view inside Window 2 / Editor - Scratchpad.
fn draw_editor_client_body<S: Surface + ?Sized>(s: &mut S, client: Rect, _palette: &ThemePalette) {
    let font_mono = Font::mono();
    let font_small = Font::small();

    // 1. Fill main background
    let bg_color = Color::rgb(13, 17, 23); // #0D1117
    s.fill_rect(client, bg_color);

    // 2. Toolbar (height 24px)
    let tb_h = 24;
    let tb_rect = Rect::new(client.x, client.y, client.w, tb_h);
    s.fill_rect(tb_rect, Color::rgb(22, 27, 34)); // #161B22
    s.fill_rect(Rect::new(client.x, client.y + tb_h - 1, client.w, 1), Color::rgb(48, 54, 61));

    // Toolbar buttons: [New] [Open] [Save] [Cut] [Copy] [Paste] [Font: Mono]
    let buttons: [(&str, i32); 7] = [
        ("New", 36),
        ("Open", 42),
        ("Save", 42),
        ("Cut", 36),
        ("Copy", 42),
        ("Paste", 46),
        ("Font: Mono", 84),
    ];

    let mut bx = client.x + 6;
    for (lbl, bw) in &buttons {
        if bx + bw > client.right() - 6 {
            break;
        }
        let b_rect = Rect::new(bx, client.y + 3, *bw, tb_h - 6);
        s.fill_rect(b_rect, Color::rgb(33, 38, 45));
        s.fill_rect(Rect::new(b_rect.x, b_rect.y, b_rect.w, 1), Color::rgb(56, 68, 77));
        s.fill_rect(Rect::new(b_rect.x, b_rect.bottom() - 1, b_rect.w, 1), Color::rgb(56, 68, 77));
        s.fill_rect(Rect::new(b_rect.x, b_rect.y, 1, b_rect.h), Color::rgb(56, 68, 77));
        s.fill_rect(Rect::new(b_rect.right() - 1, b_rect.y, 1, b_rect.h), Color::rgb(56, 68, 77));

        let tx = b_rect.x + (b_rect.w - (lbl.len() as i32 * 8)) / 2;
        let ty = b_rect.y + (b_rect.h - 8) / 2;
        ui::font::draw_text_font(s, tx, ty, lbl, Color::rgb(240, 246, 252), &font_small);
        bx += bw + 6;
    }

    // 3. Status Bar at bottom
    let sb_h = 18;
    let sb_y = client.bottom() - sb_h;
    if sb_y > client.y + tb_h + 30 {
        let sb_rect = Rect::new(client.x, sb_y, client.w, sb_h);
        s.fill_rect(sb_rect, Color::rgb(22, 27, 34));
        s.fill_rect(Rect::new(client.x, sb_y, client.w, 1), Color::rgb(48, 54, 61));
        ui::font::draw_text_font(s, client.x + 8, sb_y + 3, "HELLO.TXT *", Color::rgb(240, 246, 252), &font_small);
        let coord_str = "Ln 2, Col 18 | UTF-8 | 8x16 Mono";
        let c_sz = font_small.measure_str(coord_str);
        ui::font::draw_text_font(s, client.right() - c_sz.w - 10, sb_y + 3, coord_str, Color::rgb(88, 166, 255), &font_small);
    }

    // 4. Gutter & Document Canvas
    let gutter_w = 38;
    let doc_y = client.y + tb_h;
    let doc_h = (sb_y - doc_y).max(0);

    // Gutter background & border
    s.fill_rect(Rect::new(client.x, doc_y, gutter_w, doc_h), Color::rgb(18, 22, 28));
    s.fill_rect(Rect::new(client.x + gutter_w - 1, doc_y, 1, doc_h), Color::rgb(48, 54, 61));

    let line_h = 16;
    let text_x = client.x + gutter_w + 6;

    // Line 1: // NovaOS Scratchpad Editor
    let y1 = doc_y + 6;
    if y1 + line_h < sb_y {
        ui::font::draw_text_font(s, client.x + 16, y1, "1", Color::rgb(110, 118, 129), &font_small);
        ui::font::draw_text_font(s, text_x, y1, "// NovaOS Scratchpad Editor", Color::rgb(139, 148, 158), &font_mono);
    }

    // Line 2: Active line with highlight
    let y2 = y1 + line_h;
    if y2 + line_h < sb_y {
        s.fill_rect(Rect::new(client.x + gutter_w, y2, client.w - gutter_w, line_h), Color::rgb(22, 27, 34));
        ui::font::draw_text_font(s, client.x + 16, y2, "2", Color::rgb(240, 246, 252), &font_small);

        // fn main() {
        ui::font::draw_text_font(s, text_x, y2, "fn", Color::rgb(255, 123, 114), &font_mono);
        ui::font::draw_text_font(s, text_x + 20, y2, "main() {", Color::rgb(240, 246, 252), &font_mono);

        // Cursor at column 10
        s.fill_rect(Rect::new(text_x + 88, y2 + 1, 2, line_h - 2), Color::rgb(88, 166, 255));
    }

    // Line 3: let msg = "Hello from NovaOS!";
    let y3 = y2 + line_h;
    if y3 + line_h < sb_y {
        ui::font::draw_text_font(s, client.x + 16, y3, "3", Color::rgb(110, 118, 129), &font_small);
        ui::font::draw_text_font(s, text_x + 16, y3, "let", Color::rgb(255, 123, 114), &font_mono);
        ui::font::draw_text_font(s, text_x + 44, y3, "msg = ", Color::rgb(240, 246, 252), &font_mono);
        ui::font::draw_text_font(s, text_x + 92, y3, "\"Hello from NovaOS!\";", Color::rgb(126, 231, 135), &font_mono);
    }

    // Line 4: }
    let y4 = y3 + line_h;
    if y4 + line_h < sb_y {
        ui::font::draw_text_font(s, client.x + 16, y4, "4", Color::rgb(110, 118, 129), &font_small);
        ui::font::draw_text_font(s, text_x, y4, "}", Color::rgb(240, 246, 252), &font_mono);
    }
}

/// Renders the Software Store & App Launcher client view inside Window 3 / Software Store - Packages.
fn draw_store_client_body<S: Surface + ?Sized>(s: &mut S, client: Rect, palette: &ThemePalette) {
    let font_regular = Font::regular();
    let font_small = Font::small();

    // 1. Dark background
    let bg_color = Color::rgb(13, 17, 23);
    s.fill_rect(client, bg_color);

    // 2. Tab strip
    let tb_h = 24;
    let tb_rect = Rect::new(client.x, client.y, client.w, tb_h);
    s.fill_rect(tb_rect, Color::rgb(22, 27, 34));
    s.fill_rect(Rect::new(client.x, client.y + tb_h - 1, client.w, 1), Color::rgb(48, 54, 61));

    // Tabs: [All Packages] [Installed] [Repo Server] [Security]
    let tabs: [(&str, i32, bool); 4] = [
        ("All Packages", 84, true),
        ("Installed", 68, false),
        ("Repo Server", 76, false),
        ("Security", 60, false),
    ];
    let mut bx = client.x + 8;
    for (lbl, bw, is_active) in &tabs {
        if bx + bw > client.right() - 8 { break; }
        if *is_active {
            s.fill_rect(Rect::new(bx, client.y + 2, *bw, tb_h - 4), Color::rgb(33, 38, 45));
            s.fill_rect(Rect::new(bx, client.y + tb_h - 2, *bw, 2), palette.accent_primary);
            ui::font::draw_text_font(s, bx + 6, client.y + 7, lbl, Color::WHITE, &font_small);
        } else {
            ui::font::draw_text_font(s, bx + 6, client.y + 7, lbl, palette.text_muted, &font_small);
        }
        bx += bw + 6;
    }

    // 3. Status Bar at bottom
    let sb_h = 18;
    let sb_y = client.bottom() - sb_h;
    if sb_y > client.y + tb_h + 30 {
        let sb_rect = Rect::new(client.x, sb_y, client.w, sb_h);
        s.fill_rect(sb_rect, Color::rgb(22, 27, 34));
        s.fill_rect(Rect::new(client.x, sb_y, client.w, 1), Color::rgb(48, 54, 61));
        ui::font::draw_text_font(s, client.x + 8, sb_y + 3, "6 packages available | 2 installed", palette.accent_primary, &font_small);
        let repo_str = "HTTP Repo: 8080 Active | HMAC-SHA256";
        let r_sz = font_small.measure_str(repo_str);
        ui::font::draw_text_font(s, client.right() - r_sz.w - 10, sb_y + 3, repo_str, palette.status_success, &font_small);
    }

    // 4. Package rows
    let row_y = client.y + tb_h + 6;
    let row_h = 36;
    let rows: [(&str, &str, &str, bool, Color); 3] = [
        ("Editor", "v1.0", "Tiny text editor (demo package)", true, Color::rgb(88, 166, 255)),
        ("Game", "v2.1", "Tiny game (demo package)", false, Color::rgb(245, 180, 50)),
        ("Calc", "v1.2", "Scientific calculator utility", false, Color::rgb(57, 211, 83)),
    ];

    for (i, (name, ver, desc, is_installed, col)) in rows.iter().enumerate() {
        let ry = row_y + (i as i32 * (row_h + 4));
        if ry + row_h >= sb_y { break; }

        let r_rect = Rect::new(client.x + 8, ry, client.w - 16, row_h);
        s.fill_rect(r_rect, Color::rgb(22, 27, 34));
        s.fill_rect(Rect::new(r_rect.x, r_rect.y, 1, row_h), Color::rgb(48, 54, 61));
        s.fill_rect(Rect::new(r_rect.right() - 1, r_rect.y, 1, row_h), Color::rgb(48, 54, 61));
        s.fill_rect(Rect::new(r_rect.x, r_rect.y, r_rect.w, 1), Color::rgb(48, 54, 61));
        s.fill_rect(Rect::new(r_rect.x, r_rect.bottom() - 1, r_rect.w, 1), Color::rgb(48, 54, 61));

        // Color indicator pill
        s.fill_rect(Rect::new(r_rect.x + 4, ry + 4, 3, row_h - 8), *col);

        // App Name & Version
        ui::font::draw_text_font(s, r_rect.x + 12, ry + 6, name, Color::WHITE, &font_regular);
        ui::font::draw_text_font(s, r_rect.x + 64, ry + 8, ver, palette.text_muted, &font_small);

        // Description
        ui::font::draw_text_font(s, r_rect.x + 12, ry + 22, desc, palette.text_muted, &font_small);

        // Verified Badge
        let badge_x = r_rect.right() - 160;
        if badge_x > r_rect.x + 120 {
            ui::font::draw_text_font(s, badge_x, ry + 6, "[✓] Verified", palette.status_success, &font_small);
        }

        // Action Button
        let btn_w = 46;
        let btn_h = 18;
        let btn_x = r_rect.right() - btn_w - 8;
        let btn_y = ry + (row_h - btn_h) / 2;
        let b_rect = Rect::new(btn_x, btn_y, btn_w, btn_h);

        if *is_installed {
            s.fill_rect(b_rect, palette.status_success);
            ui::font::draw_text_font(s, btn_x + 6, btn_y + 3, "Launch", Color::WHITE, &font_small);
        } else {
            s.fill_rect(b_rect, palette.accent_primary);
            ui::font::draw_text_font(s, btn_x + 6, btn_y + 3, "Install", Color::WHITE, &font_small);
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

    // Case 9 (Month 3 Core desktop apps): Persistent top menu bar and app menu dispatch
    let mut wm_mb = Wm::new_for_display(1024, 768);
    wm_mb.raise(0); // Focus Window 0 (Terminal)

    // 1. Verify menubar title reflects active focused window
    let mb_term = wm_mb.current_menubar();
    let is_terminal = mb_term.app_name == "Terminal" && mb_term.menus.len() == 4;

    wm_mb.raise(1); // Focus Window 1 (Files)
    let mb_files = wm_mb.current_menubar();
    let is_files = mb_files.app_name == "Files" && mb_files.menus.len() == 4;

    wm_mb.raise(0); // Restore Window 0 (Terminal)

    // 2. Locate File menu item (menu 0)
    let font = Font::regular();
    let star_size = 12;
    let star_x = 8;
    let app_sz = font.measure_str("Terminal");
    let start_x = star_x + star_size + 8 + app_sz.w + 18;
    let file_rect = ui::menu::measure_menubar_item(&mb_term, 0, start_x, 0, wm_mb.menubar_h, &font)
        .unwrap_or(Rect::new(0, 0, 0, 0));

    let file_hit = wm_mb.menubar_item_at(file_rect.x + 4, file_rect.y + 4) == Some(0);
    let shape_menu = wm_mb.cursor_shape_at(file_rect.x + 4, file_rect.y + 4);

    // 3. Click File menu item to open dropdown
    wm_mb.cursor_x = file_rect.x + 4;
    wm_mb.cursor_y = file_rect.y + 4;
    wm_mb.apply_mouse(mouse(0, 0, true));
    wm_mb.apply_mouse(mouse(0, 0, false));
    let dropdown_opened = wm_mb.active_menu == Some(0) && wm_mb.active_dropdown_rect().is_some();

    // 4. Locate and click "Quit Terminal" item (item 4, action 105)
    let drop_rect = wm_mb.active_dropdown_rect().unwrap_or(Rect::new(0, 0, 0, 0));
    let quit_item_y = drop_rect.y + 4 + 22 + 22 + 6 + 22 + 10;
    let quit_hit = wm_mb.dropdown_item_at(drop_rect.x + 10, quit_item_y) == Some(4);
    let shape_drop_item = wm_mb.cursor_shape_at(drop_rect.x + 10, quit_item_y);

    wm_mb.cursor_x = drop_rect.x + 10;
    wm_mb.cursor_y = quit_item_y;
    wm_mb.apply_mouse(mouse(0, 0, true));
    wm_mb.apply_mouse(mouse(0, 0, false));

    let dropdown_closed = wm_mb.active_menu.is_none();
    let menu_event = wm_mb.take_menu_event();
    let event_ok = menu_event == Some(MenuClickEvent {
        window_id: 0,
        menu_index: 0,
        item_index: 4,
        action_id: ui::menu::ACTION_FILE_QUIT,
    });
    let event_drained = wm_mb.take_menu_event().is_none();

    // 5. Render menubar into BufferSurface32
    test_buf[..1024 * 30].fill(0);
    let mut menu_surf = ui::draw::BufferSurface32::new(&mut test_buf[..1024 * 30], 1024, 30);
    wm_mb.render(&mut menu_surf);

    let mb_bg = palette.surface_sunken.to_xrgb();
    let p_corner = menu_surf.buffer[0];
    let p_divider = menu_surf.buffer[23 * 1024 + 100];
    let divider_ok = p_divider == palette.border_subtle.to_xrgb();
    let bg_ok = p_corner == mb_bg;

    if is_terminal && is_files && file_hit && shape_menu == CursorShape::Pointer
        && dropdown_opened && quit_hit && shape_drop_item == CursorShape::Pointer
        && dropdown_closed && event_ok && event_drained && bg_ok && divider_ok
    {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: persistent top menu bar and app menu dispatch\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: persistent top menu bar\n\0".as_ptr()) };
        code |= 512;
    }

    // Case 10 (Month 3 Core desktop apps): Dock with app pinning and window switching
    let mut wm_dock = Wm::new_for_display(1024, 768);

    // 1. Verify default pinned applications
    let p_term = wm_dock.is_app_pinned(APP_ID_TERMINAL);
    let p_files = wm_dock.is_app_pinned(APP_ID_FILES);
    let p_edit = wm_dock.is_app_pinned(APP_ID_EDITOR);
    let p_sys = wm_dock.is_app_pinned(APP_ID_SYSTEM);
    let p_set = wm_dock.is_app_pinned(APP_ID_SETTINGS);
    let initial_count_ok = wm_dock.dock_count == 5;

    // 2. Verify window switching and minimize/restore toggle
    // Initially Window 3 (System) is active (topmost in z_order)
    let init_active = wm_dock.active_window() == Some(3);
    // Clicking active Window 3 item in dock minimizes it!
    wm_dock.on_dock_item_click(3);
    let win3_minimized = wm_dock.windows[3].is_minimized;
    let min_event_ok = wm_dock.take_dock_event() == Some(DockEvent::Minimize { window_id: 3 });

    // Clicking minimized Window 3 item in dock restores and raises it!
    wm_dock.on_dock_item_click(3);
    let win3_restored = !wm_dock.windows[3].is_minimized && wm_dock.active_window() == Some(3);
    let res_event_ok = wm_dock.take_dock_event() == Some(DockEvent::Restore { window_id: 3 });

    // 3. Focus switching between open windows
    // Clicking Window 1 (Files) raises it
    wm_dock.on_dock_item_click(1);
    let win1_focused = wm_dock.active_window() == Some(1);
    let switch_event_ok = wm_dock.take_dock_event() == Some(DockEvent::Switch { window_id: 1 });

    // 4. Launch-on-click for pinned unlaunched app (Settings, App 5)
    let set_init_running = wm_dock.dock_items()[4].unwrap().is_running;
    wm_dock.on_dock_item_click(4);
    let launch_event_ok = wm_dock.take_dock_event() == Some(DockEvent::Launch {
        app_id: APP_ID_SETTINGS,
        exec_path: "SETTINGS.ELF",
    });

    // 5. Window close and re-launch
    // Close Window 2 (Editor)
    wm_dock.close_window(2);
    let win2_closed = !wm_dock.windows[2].is_open;
    let edit_pinned_after_close = wm_dock.is_app_pinned(APP_ID_EDITOR);
    let edit_not_running = !wm_dock.dock_items()[2].unwrap().is_running;

    // Launch Editor by clicking its dock item
    wm_dock.on_dock_item_click(2);
    let win2_reopened = wm_dock.windows[2].is_open && wm_dock.active_window() == Some(2);
    let relaunch_event_ok = wm_dock.take_dock_event() == Some(DockEvent::Launch {
        app_id: APP_ID_EDITOR,
        exec_path: "EDIT.ELF",
    });

    // 6. Dynamic Pinning and Unpinning
    let custom_app = DockApp::new(6, "Calc", "CALC.ELF", true);
    let pin_ok = wm_dock.pin_app(custom_app) && wm_dock.is_app_pinned(6) && wm_dock.dock_count == 6;
    let unpin_ok = wm_dock.unpin_app(6) && !wm_dock.is_app_pinned(6) && wm_dock.dock_count == 5;

    // 7. Dock surface rasterization
    test_buf[..1024 * 40].fill(0);
    let mut dock_surf = ui::draw::BufferSurface32::new(&mut test_buf[..1024 * 40], 1024, 40);
    wm_dock.render(&mut dock_surf);
    let dock_non_zero = dock_surf.buffer.iter().filter(|&&p| p != 0).count();
    let render_ok = dock_non_zero > 100;

    if p_term && p_files && p_edit && p_sys && p_set && initial_count_ok
        && init_active && win3_minimized && min_event_ok && win3_restored && res_event_ok
        && win1_focused && switch_event_ok && !set_init_running && launch_event_ok
        && win2_closed && edit_pinned_after_close && edit_not_running && win2_reopened
        && relaunch_event_ok && pin_ok && unpin_ok && render_ok
    {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: dock with app pinning and window switching\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: dock with app pinning and window switching\n\0".as_ptr()) };
        code |= 1024;
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

    #[test]
    fn test_wm_menubar_interaction() {
        let mut wm = Wm::new_for_display(1024, 768);
        wm.raise(0); // Terminal

        assert_eq!(wm.current_menubar().app_name, "Terminal");
        assert_eq!(wm.menubar_h, 24);
        assert!(wm.active_menu.is_none());

        // Find File menu
        let font = Font::regular();
        let star_size = 12;
        let star_x = 8;
        let app_sz = font.measure_str("Terminal");
        let start_x = star_x + star_size + 8 + app_sz.w + 18;
        let file_rect = ui::menu::measure_menubar_item(&wm.current_menubar(), 0, start_x, 0, wm.menubar_h, &font).unwrap();

        // Hover over File menu
        wm.cursor_x = file_rect.x + 2;
        wm.cursor_y = file_rect.y + 2;
        wm.apply_mouse(mouse(0, 0, false));
        assert_eq!(wm.hovered_menu, Some(0));
        assert_eq!(wm.cursor_shape_at(wm.cursor_x, wm.cursor_y), CursorShape::Pointer);

        // Click to open File menu
        wm.apply_mouse(mouse(0, 0, true));
        wm.apply_mouse(mouse(0, 0, false));
        assert_eq!(wm.active_menu, Some(0));
        assert!(wm.active_dropdown_rect().is_some());

        // Switch menu by hovering over Edit menu (menu 1)
        let edit_rect = ui::menu::measure_menubar_item(&wm.current_menubar(), 1, start_x, 0, wm.menubar_h, &font).unwrap();
        wm.cursor_x = edit_rect.x + 2;
        wm.cursor_y = edit_rect.y + 2;
        wm.apply_mouse(mouse(0, 0, false));
        assert_eq!(wm.active_menu, Some(1));

        // Switch back to File menu
        wm.cursor_x = file_rect.x + 2;
        wm.cursor_y = file_rect.y + 2;
        wm.apply_mouse(mouse(0, 0, false));
        assert_eq!(wm.active_menu, Some(0));

        // Click "Close Window" (item 3, action 104)
        let drop_rect = wm.active_dropdown_rect().unwrap();
        let close_y = drop_rect.y + 4 + 22 + 22 + 6 + 10;
        wm.cursor_x = drop_rect.x + 10;
        wm.cursor_y = close_y;
        wm.apply_mouse(mouse(0, 0, true));
        wm.apply_mouse(mouse(0, 0, false));

        assert!(wm.active_menu.is_none());
        let ev = wm.take_menu_event().expect("should emit menu event");
        assert_eq!(ev.window_id, 0);
        assert_eq!(ev.menu_index, 0);
        assert_eq!(ev.item_index, 3);
        assert_eq!(ev.action_id, ui::menu::ACTION_FILE_CLOSE);
    }

    #[test]
    fn test_wm_dock_pinning_and_switching() {
        let mut wm = Wm::new_for_display(1024, 768);
        assert_eq!(wm.dock_count, 5);
        assert!(wm.is_app_pinned(APP_ID_TERMINAL));
        assert!(wm.is_app_pinned(APP_ID_SETTINGS));

        // Window 3 (System) starts active
        assert_eq!(wm.active_window(), Some(3));

        // Click active Window 3 dock button -> minimizes
        wm.on_dock_item_click(3);
        assert!(wm.windows[3].is_minimized);
        assert_eq!(wm.take_dock_event(), Some(DockEvent::Minimize { window_id: 3 }));

        // Click again -> restores and raises
        wm.on_dock_item_click(3);
        assert!(!wm.windows[3].is_minimized);
        assert_eq!(wm.active_window(), Some(3));
        assert_eq!(wm.take_dock_event(), Some(DockEvent::Restore { window_id: 3 }));

        // Switch to Window 0 (Terminal)
        wm.on_dock_item_click(0);
        assert_eq!(wm.active_window(), Some(0));
        assert_eq!(wm.take_dock_event(), Some(DockEvent::Switch { window_id: 0 }));
    }

    #[test]
    fn test_wm_dock_launch_on_click() {
        let mut wm = Wm::new_for_display(1024, 768);

        // Click Settings (item 4, not running) -> launches Settings!
        let items = wm.dock_items();
        assert!(!items[4].unwrap().is_running);
        wm.on_dock_item_click(4);

        let ev = wm.take_dock_event().expect("should emit launch event");
        assert_eq!(ev, DockEvent::Launch { app_id: APP_ID_SETTINGS, exec_path: "SETTINGS.ELF" });

        // Close Window 1 (Files)
        wm.close_window(1);
        assert!(!wm.windows[1].is_open);
        assert!(wm.is_app_pinned(APP_ID_FILES));

        // Click Files dock item (item 1) -> launches and reopens window
        wm.on_dock_item_click(1);
        assert!(wm.windows[1].is_open);
        assert_eq!(wm.active_window(), Some(1));
    }
}
