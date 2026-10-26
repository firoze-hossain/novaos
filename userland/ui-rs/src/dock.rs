//! # Desktop Dock & Taskbar System with App Pinning and Window Switching
//!
//! Provides data structures, state management, and rendering routines for the
//! NovaOS desktop dock / taskbar. Supports pinned launchable applications,
//! active running window switching, minimize/restore state transitions,
//! glowing running indicator pips, and launch-on-click dispatch.

use crate::color::{Color, ThemePalette, WHITE};
use crate::draw::{draw_line, draw_rect_outline, draw_rounded_rect, Surface};
use crate::font::{draw_text_font, Font};
use crate::geom::Rect;
use crate::spacing::*;

/// Standard application identifiers for core desktop applications.
pub const APP_ID_TERMINAL: u8 = 1;
pub const APP_ID_FILES: u8    = 2;
pub const APP_ID_EDITOR: u8   = 3;
pub const APP_ID_SYSTEM: u8   = 4;
pub const APP_ID_SETTINGS: u8 = 5;
pub const APP_ID_STORE: u8    = 6;

pub const MAX_PINNED_APPS: usize = 8;

/// A registered application that can be pinned to the desktop dock / taskbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DockApp {
    pub app_id: u8,
    pub name: &'static str,
    pub exec_path: &'static str,
    pub is_pinned: bool,
}

impl DockApp {
    pub const fn new(app_id: u8, name: &'static str, exec_path: &'static str, is_pinned: bool) -> Self {
        DockApp {
            app_id,
            name,
            exec_path,
            is_pinned,
        }
    }
}

/// Default suite of pinned applications for NovaOS desktop.
pub const DEFAULT_PINNED_APPS: [DockApp; 5] = [
    DockApp::new(APP_ID_TERMINAL, "Terminal", "SHELL.ELF",    true),
    DockApp::new(APP_ID_FILES,    "Files",    "FILES.ELF",    true),
    DockApp::new(APP_ID_EDITOR,   "Editor",   "EDIT.ELF",     true),
    DockApp::new(APP_ID_SYSTEM,   "System",   "SYSINFO.ELF",  true),
    DockApp::new(APP_ID_SETTINGS, "Settings", "SETTINGS.ELF", true),
];

/// Returns the standard DockApp definition for a given app ID, if known.
pub fn app_for_id(id: u8) -> Option<DockApp> {
    match id {
        APP_ID_TERMINAL => Some(DEFAULT_PINNED_APPS[0]),
        APP_ID_FILES    => Some(DEFAULT_PINNED_APPS[1]),
        APP_ID_EDITOR   => Some(DEFAULT_PINNED_APPS[2]),
        APP_ID_SYSTEM   => Some(DEFAULT_PINNED_APPS[3]),
        APP_ID_SETTINGS => Some(DEFAULT_PINNED_APPS[4]),
        APP_ID_STORE    => Some(DockApp::new(APP_ID_STORE, "Store", "STORE.ELF", true)),
        _ => None,
    }
}

/// The runtime state of an item in the dock / taskbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DockItem {
    pub app: DockApp,
    pub is_running: bool,
    pub is_active: bool,
    pub is_minimized: bool,
    pub window_id: Option<usize>,
}

impl DockItem {
    pub const fn new(
        app: DockApp,
        is_running: bool,
        is_active: bool,
        is_minimized: bool,
        window_id: Option<usize>,
    ) -> Self {
        DockItem {
            app,
            is_running,
            is_active,
            is_minimized,
            window_id,
        }
    }
}

/// Action / Event emitted when a dock or taskbar item is activated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockEvent {
    /// Launch a pinned app that is not currently running.
    Launch { app_id: u8, exec_path: &'static str },
    /// Switch focus to an already running window.
    Switch { window_id: usize },
    /// Minimize an active focused window to the taskbar.
    Minimize { window_id: usize },
    /// Restore / un-minimize a minimized window back to the desktop.
    Restore { window_id: usize },
    /// App dynamically pinned to the dock.
    Pin { app_id: u8 },
    /// App unpinned from the dock.
    Unpin { app_id: u8 },
}

// ---------------------------------------------------------------------------
// Layout and Rendering Routines
// ---------------------------------------------------------------------------

/// Computes the bounding rectangle for a dock item at index `idx`.
pub fn measure_dock_item(
    idx: usize,
    total_items: usize,
    taskbar_rect: Rect,
    start_offset_x: i32,
    max_item_w: i32,
) -> Option<Rect> {
    if idx >= total_items || total_items == 0 || taskbar_rect.w <= 0 {
        return None;
    }
    let available_w = taskbar_rect.w - start_offset_x - 120; // reserve space for clock on right
    let item_w = (available_w / total_items as i32).min(max_item_w).max(48);
    let x = taskbar_rect.x + start_offset_x + (idx as i32 * (item_w + 4));
    let y = taskbar_rect.y + 2;
    let h = taskbar_rect.h - 4;
    Some(Rect::new(x, y, item_w, h))
}

/// Renders a single app icon glyph into surface `s`.
pub fn draw_dock_app_icon<S: Surface + ?Sized>(s: &mut S, x: i32, y: i32, app_id: u8, palette: &ThemePalette) {
    match app_id {
        APP_ID_TERMINAL => {
            // Terminal prompt `>_`
            draw_rounded_rect(s, Rect::new(x, y, 14, 14), 2, palette.surface_sunken);
            s.fill_rect(Rect::new(x + 2, y + 3, 2, 2), palette.status_success);
            s.fill_rect(Rect::new(x + 4, y + 5, 2, 2), palette.status_success);
            s.fill_rect(Rect::new(x + 2, y + 7, 2, 2), palette.status_success);
            s.fill_rect(Rect::new(x + 7, y + 9, 4, 2), WHITE);
        }
        APP_ID_FILES => {
            // Folder icon
            let folder_col = Color::rgb(245, 180, 50);
            s.fill_rect(Rect::new(x + 2, y + 2, 5, 3), folder_col.lighten(25));
            draw_rounded_rect(s, Rect::new(x + 1, y + 4, 12, 8), 1, folder_col);
            s.fill_rect(Rect::new(x + 3, y + 6, 8, 1), WHITE);
        }
        APP_ID_EDITOR => {
            // Document sheet icon
            let doc_col = WHITE;
            draw_rounded_rect(s, Rect::new(x + 2, y + 1, 10, 12), 1, doc_col);
            s.put_pixel(x + 10, y + 1, palette.surface_sunken);
            s.fill_rect(Rect::new(x + 4, y + 4, 6, 1), palette.text_muted);
            s.fill_rect(Rect::new(x + 4, y + 6, 6, 1), palette.text_muted);
            s.fill_rect(Rect::new(x + 4, y + 8, 4, 1), palette.text_muted);
        }
        APP_ID_SETTINGS => {
            // Sliders / Preferences icon
            let set_col = Color::rgb(180, 190, 205);
            draw_rounded_rect(s, Rect::new(x + 1, y + 1, 12, 12), 2, palette.surface_sunken);
            // Track 1
            s.fill_rect(Rect::new(x + 3, y + 4, 8, 1), set_col);
            s.fill_rect(Rect::new(x + 5, y + 3, 3, 3), palette.accent_primary);
            // Track 2
            s.fill_rect(Rect::new(x + 3, y + 9, 8, 1), set_col);
            s.fill_rect(Rect::new(x + 8, y + 8, 3, 3), palette.status_success);
        }
        APP_ID_STORE => {
            // Shopping bag / package icon
            let bag_col = Color::rgb(137, 87, 229);
            draw_rounded_rect(s, Rect::new(x + 1, y + 4, 12, 9), 1, bag_col);
            draw_line(s, x + 4, y + 4, x + 4, y + 2, WHITE);
            draw_line(s, x + 4, y + 2, x + 10, y + 2, WHITE);
            draw_line(s, x + 10, y + 2, x + 10, y + 4, WHITE);
            s.fill_rect(Rect::new(x + 6, y + 5, 2, 8), WHITE);
        }
        _ => {
            // System Gear icon
            let gear_col = Color::rgb(140, 160, 180);
            s.fill_rect(Rect::new(x + 4, y + 2, 6, 10), gear_col);
            s.fill_rect(Rect::new(x + 2, y + 4, 10, 6), gear_col);
            s.fill_rect(Rect::new(x + 5, y + 5, 4, 4), palette.titlebar_active);
            s.fill_rect(Rect::new(x + 6, y + 6, 2, 2), WHITE);
        }
    }
}

/// Renders the complete dock / taskbar surface across the bottom of the screen.
pub fn draw_dock_surface<S: Surface + ?Sized>(
    s: &mut S,
    taskbar_rect: Rect,
    items: &[DockItem],
    hovered_idx: Option<usize>,
    clock_str: &str,
    palette: &ThemePalette,
) {
    if taskbar_rect.w <= 0 || taskbar_rect.h <= 0 {
        return;
    }

    // 1. Taskbar background & top highlight line
    s.fill_rect(taskbar_rect, palette.taskbar_bg);
    s.fill_rect(Rect::new(taskbar_rect.x, taskbar_rect.y, taskbar_rect.w, 1), palette.border_subtle);

    let font = if taskbar_rect.h >= 24 { Font::regular() } else { Font::small() };
    let line_h = font.line_height();

    // 2. NovaOS Start / Launcher badge on far left
    let start_w = if taskbar_rect.h >= 24 { 36 } else { 24 };
    let start_rect = Rect::new(taskbar_rect.x + 4, taskbar_rect.y + 3, start_w, taskbar_rect.h - 6);
    draw_rounded_rect(s, start_rect, RADIUS_XS, palette.surface_raised);
    draw_rect_outline(s, start_rect, 1, palette.border_focused);

    let star_size = if taskbar_rect.h >= 24 { 12 } else { 8 };
    let star_x = start_rect.x + (start_rect.w - star_size) / 2;
    let star_y = start_rect.y + (start_rect.h - star_size) / 2;
    s.fill_rect(Rect::new(star_x + 3, star_y, star_size - 6, star_size), palette.accent_primary);
    s.fill_rect(Rect::new(star_x, star_y + 3, star_size, star_size - 6), palette.accent_primary);
    s.fill_rect(Rect::new(star_x + 2, star_y + 2, star_size - 4, star_size - 4), WHITE);

    // 3. Dock Items (pinned apps & running windows)
    let start_offset_x = start_w + 12;
    let max_item_w = if taskbar_rect.w >= 800 { 120 } else { 80 };

    for (idx, item) in items.iter().enumerate() {
        if let Some(rect) = measure_dock_item(idx, items.len(), taskbar_rect, start_offset_x, max_item_w) {
            let is_hovered = hovered_idx == Some(idx);

            // Item background pill
            let bg_color = if item.is_active {
                palette.taskbar_item_active
            } else if is_hovered {
                palette.surface_raised
            } else if item.is_minimized {
                palette.surface_sunken
            } else {
                palette.taskbar_item_inactive
            };

            draw_rounded_rect(s, rect, RADIUS_SM, bg_color);

            if item.is_active {
                draw_rect_outline(s, rect, 1, palette.border_focused);
            }

            // App Icon
            let icon_x = rect.x + 8;
            let icon_y = rect.y + (rect.h - 14) / 2;
            draw_dock_app_icon(s, icon_x, icon_y, item.app.app_id, palette);

            // App Name Text (if space permits)
            if rect.w >= 70 {
                let text_x = icon_x + 18;
                let text_y = rect.y + (rect.h - line_h) / 2;
                let text_col = if item.is_active {
                    palette.text_primary
                } else if item.is_running {
                    palette.text_secondary
                } else {
                    palette.text_muted
                };
                draw_text_font(s, text_x, text_y, item.app.name, text_col, &font);
            }

            // Running Indicator Pip / Bar at bottom of dock item
            if item.is_active {
                // Active focused window: bold accent bar
                s.fill_rect(
                    Rect::new(rect.x + 8, rect.bottom() - 2, rect.w - 16, 2),
                    palette.accent_primary,
                );
            } else if item.is_running {
                if item.is_minimized {
                    // Minimized running window: hollow pip
                    draw_rect_outline(
                        s,
                        Rect::new(rect.x + rect.w / 2 - 3, rect.bottom() - 3, 6, 2),
                        1,
                        palette.text_muted,
                    );
                } else {
                    // Running background window: solid glowing pip
                    s.fill_rect(
                        Rect::new(rect.x + rect.w / 2 - 3, rect.bottom() - 3, 6, 2),
                        palette.text_secondary,
                    );
                }
            } else if item.app.is_pinned {
                // Pinned launcher (not currently running): small subtle dot
                s.fill_rect(
                    Rect::new(rect.x + rect.w / 2 - 1, rect.bottom() - 2, 2, 1),
                    palette.border_subtle,
                );
            }
        }
    }

    // 4. Clock and Status Indicator on far right
    let clock_sz = font.measure_str(clock_str);
    let clock_x = taskbar_rect.right() - clock_sz.w - 14;
    let clock_y = taskbar_rect.y + (taskbar_rect.h - line_h) / 2;
    draw_text_font(s, clock_x, clock_y, clock_str, palette.text_primary, &font);

    // Green online status pip next to clock
    let dot_x = clock_x - 12;
    let dot_y = taskbar_rect.y + (taskbar_rect.h - 6) / 2;
    draw_rounded_rect(s, Rect::new(dot_x, dot_y, 6, 6), 3, palette.status_success);
}
