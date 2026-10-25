//! # Desktop Menu Bar and Context Menu System for NovaOS
//!
//! Provides data structures and drawing routines for persistent top-of-screen
//! application menu bars, dropdown action menus, keyboard accelerators, and
//! event dispatch back to focused client applications.

use crate::color::{Color, ThemePalette, WHITE};
use crate::draw::{draw_rect_outline, draw_rounded_rect, Surface};
use crate::font::{draw_text_font, Font};
use crate::geom::Rect;
use crate::spacing::*;

/// Standard Action IDs for common desktop menu commands.
pub const ACTION_APP_ABOUT: u32       = 100;
pub const ACTION_FILE_NEW: u32        = 101;
pub const ACTION_FILE_OPEN: u32       = 102;
pub const ACTION_FILE_SAVE: u32       = 103;
pub const ACTION_FILE_CLOSE: u32      = 104;
pub const ACTION_FILE_QUIT: u32       = 105;

pub const ACTION_EDIT_UNDO: u32       = 201;
pub const ACTION_EDIT_REDO: u32       = 202;
pub const ACTION_EDIT_CUT: u32        = 203;
pub const ACTION_EDIT_COPY: u32       = 204;
pub const ACTION_EDIT_PASTE: u32      = 205;
pub const ACTION_EDIT_SELECT_ALL: u32 = 206;

pub const ACTION_VIEW_ZOOM_IN: u32    = 301;
pub const ACTION_VIEW_ZOOM_OUT: u32   = 302;
pub const ACTION_VIEW_FULLSCREEN: u32 = 303;

pub const ACTION_WINDOW_MINIMIZE: u32 = 401;
pub const ACTION_WINDOW_MAXIMIZE: u32 = 402;
pub const ACTION_WINDOW_NEXT: u32     = 403;

pub const ACTION_HELP_ABOUT: u32      = 501;
pub const ACTION_HELP_DOCS: u32       = 502;

/// An individual item inside a dropdown menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub label: &'static str,
    pub shortcut: &'static str,
    pub action_id: u32,
    pub is_separator: bool,
    pub is_enabled: bool,
}

impl MenuItem {
    pub const fn action(label: &'static str, shortcut: &'static str, action_id: u32) -> Self {
        MenuItem {
            label,
            shortcut,
            action_id,
            is_separator: false,
            is_enabled: true,
        }
    }

    pub const fn disabled(label: &'static str, shortcut: &'static str, action_id: u32) -> Self {
        MenuItem {
            label,
            shortcut,
            action_id,
            is_separator: false,
            is_enabled: false,
        }
    }

    pub const fn separator() -> Self {
        MenuItem {
            label: "",
            shortcut: "",
            action_id: 0,
            is_separator: true,
            is_enabled: false,
        }
    }
}

/// A top-level menu category (e.g., "File", "Edit", "View").
#[derive(Clone, Copy, Debug)]
pub struct Menu {
    pub title: &'static str,
    pub items: &'static [MenuItem],
}

impl Menu {
    pub const fn new(title: &'static str, items: &'static [MenuItem]) -> Self {
        Menu { title, items }
    }
}

/// A complete menu bar specification for an active desktop application.
#[derive(Clone, Copy, Debug)]
pub struct MenuBar {
    pub app_name: &'static str,
    pub menus: &'static [Menu],
}

impl MenuBar {
    pub const fn new(app_name: &'static str, menus: &'static [Menu]) -> Self {
        MenuBar { app_name, menus }
    }
}

/// Event reporting a menu action click back to the focused application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuClickEvent {
    pub window_id: usize,
    pub menu_index: usize,
    pub item_index: usize,
    pub action_id: u32,
}

// ---------------------------------------------------------------------------
// Standard Application Menu Templates
// ---------------------------------------------------------------------------

const TERMINAL_FILE_ITEMS: &[MenuItem] = &[
    MenuItem::action("New Tab", "Ctrl+T", ACTION_FILE_NEW),
    MenuItem::action("New Window", "Ctrl+N", ACTION_FILE_NEW),
    MenuItem::separator(),
    MenuItem::action("Close Window", "Ctrl+W", ACTION_FILE_CLOSE),
    MenuItem::action("Quit Terminal", "Ctrl+Q", ACTION_FILE_QUIT),
];

const TERMINAL_EDIT_ITEMS: &[MenuItem] = &[
    MenuItem::action("Copy", "Ctrl+C", ACTION_EDIT_COPY),
    MenuItem::action("Paste", "Ctrl+V", ACTION_EDIT_PASTE),
    MenuItem::action("Select All", "Ctrl+A", ACTION_EDIT_SELECT_ALL),
];

const TERMINAL_VIEW_ITEMS: &[MenuItem] = &[
    MenuItem::action("Zoom In", "Ctrl++", ACTION_VIEW_ZOOM_IN),
    MenuItem::action("Zoom Out", "Ctrl+-", ACTION_VIEW_ZOOM_OUT),
    MenuItem::action("Fullscreen", "F11", ACTION_VIEW_FULLSCREEN),
];

const TERMINAL_HELP_ITEMS: &[MenuItem] = &[
    MenuItem::action("Terminal Documentation", "", ACTION_HELP_DOCS),
    MenuItem::action("About Terminal", "", ACTION_HELP_ABOUT),
];

pub const TERMINAL_MENUBAR: MenuBar = MenuBar::new("Terminal", &[
    Menu::new("File", TERMINAL_FILE_ITEMS),
    Menu::new("Edit", TERMINAL_EDIT_ITEMS),
    Menu::new("View", TERMINAL_VIEW_ITEMS),
    Menu::new("Help", TERMINAL_HELP_ITEMS),
]);

const FILES_FILE_ITEMS: &[MenuItem] = &[
    MenuItem::action("New Folder", "Ctrl+Shift+N", ACTION_FILE_NEW),
    MenuItem::action("Open", "Enter", ACTION_FILE_OPEN),
    MenuItem::separator(),
    MenuItem::action("Close Window", "Ctrl+W", ACTION_FILE_CLOSE),
];

const FILES_EDIT_ITEMS: &[MenuItem] = &[
    MenuItem::action("Cut", "Ctrl+X", ACTION_EDIT_CUT),
    MenuItem::action("Copy", "Ctrl+C", ACTION_EDIT_COPY),
    MenuItem::action("Paste", "Ctrl+V", ACTION_EDIT_PASTE),
    MenuItem::action("Select All", "Ctrl+A", ACTION_EDIT_SELECT_ALL),
];

const FILES_VIEW_ITEMS: &[MenuItem] = &[
    MenuItem::action("Icon View", "", ACTION_VIEW_ZOOM_IN),
    MenuItem::action("List View", "", ACTION_VIEW_ZOOM_OUT),
];

const FILES_HELP_ITEMS: &[MenuItem] = &[
    MenuItem::action("About Files", "", ACTION_HELP_ABOUT),
];

pub const FILES_MENUBAR: MenuBar = MenuBar::new("Files", &[
    Menu::new("File", FILES_FILE_ITEMS),
    Menu::new("Edit", FILES_EDIT_ITEMS),
    Menu::new("View", FILES_VIEW_ITEMS),
    Menu::new("Help", FILES_HELP_ITEMS),
]);

const EDITOR_FILE_ITEMS: &[MenuItem] = &[
    MenuItem::action("New Document", "Ctrl+N", ACTION_FILE_NEW),
    MenuItem::action("Open File...", "Ctrl+O", ACTION_FILE_OPEN),
    MenuItem::action("Save", "Ctrl+S", ACTION_FILE_SAVE),
    MenuItem::separator(),
    MenuItem::action("Quit Editor", "Ctrl+Q", ACTION_FILE_QUIT),
];

const EDITOR_EDIT_ITEMS: &[MenuItem] = &[
    MenuItem::action("Undo", "Ctrl+Z", ACTION_EDIT_UNDO),
    MenuItem::action("Redo", "Ctrl+Y", ACTION_EDIT_REDO),
    MenuItem::separator(),
    MenuItem::action("Cut", "Ctrl+X", ACTION_EDIT_CUT),
    MenuItem::action("Copy", "Ctrl+C", ACTION_EDIT_COPY),
    MenuItem::action("Paste", "Ctrl+V", ACTION_EDIT_PASTE),
];

const EDITOR_HELP_ITEMS: &[MenuItem] = &[
    MenuItem::action("About Editor", "", ACTION_HELP_ABOUT),
];

pub const EDITOR_MENUBAR: MenuBar = MenuBar::new("Editor", &[
    Menu::new("File", EDITOR_FILE_ITEMS),
    Menu::new("Edit", EDITOR_EDIT_ITEMS),
    Menu::new("Help", EDITOR_HELP_ITEMS),
]);

const SYSTEM_SYS_ITEMS: &[MenuItem] = &[
    MenuItem::action("System Diagnostics", "", ACTION_APP_ABOUT),
    MenuItem::action("Restart Supervisor", "", ACTION_WINDOW_NEXT),
    MenuItem::separator(),
    MenuItem::action("Shutdown", "", ACTION_FILE_QUIT),
];

const SYSTEM_VIEW_ITEMS: &[MenuItem] = &[
    MenuItem::action("Refresh", "F5", ACTION_VIEW_ZOOM_IN),
];

const SYSTEM_HELP_ITEMS: &[MenuItem] = &[
    MenuItem::action("About NovaOS", "", ACTION_HELP_ABOUT),
];

pub const SYSTEM_MENUBAR: MenuBar = MenuBar::new("System", &[
    Menu::new("System", SYSTEM_SYS_ITEMS),
    Menu::new("View", SYSTEM_VIEW_ITEMS),
    Menu::new("Help", SYSTEM_HELP_ITEMS),
]);

/// Returns standard MenuBar for a given window label (1 = Terminal, 2 = Files, 3 = Editor, 4 = System).
pub fn menubar_for_label(label: u8) -> MenuBar {
    match label {
        1 => TERMINAL_MENUBAR,
        2 => FILES_MENUBAR,
        3 => EDITOR_MENUBAR,
        _ => SYSTEM_MENUBAR,
    }
}

// ---------------------------------------------------------------------------
// Menu Bar & Dropdown Rendering Routines
// ---------------------------------------------------------------------------

/// Computes the bounding rectangle for top-level menu item at index `menu_idx`.
pub fn measure_menubar_item(
    menubar: &MenuBar,
    menu_idx: usize,
    start_x: i32,
    y: i32,
    h: i32,
    font: &Font,
) -> Option<Rect> {
    if menu_idx >= menubar.menus.len() {
        return None;
    }
    let mut cur_x = start_x;
    for (i, m) in menubar.menus.iter().enumerate() {
        let sz = font.measure_str(m.title);
        let item_w = sz.w + 14;
        if i == menu_idx {
            return Some(Rect::new(cur_x, y + 2, item_w, h - 4));
        }
        cur_x += item_w + 2;
    }
    None
}

/// Renders the persistent horizontal menu bar across the top of the display.
pub fn draw_menubar_surface<S: Surface + ?Sized>(
    s: &mut S,
    rect: Rect,
    menubar: &MenuBar,
    active_menu: Option<usize>,
    hovered_menu: Option<usize>,
    clock_str: &str,
    palette: &ThemePalette,
) {
    if rect.h <= 0 || rect.w <= 0 {
        return;
    }

    // Background fill & bottom separator line
    s.fill_rect(rect, palette.surface_sunken);
    s.fill_rect(Rect::new(rect.x, rect.bottom() - 1, rect.w, 1), palette.border_subtle);

    let font = if rect.h >= 20 { Font::regular() } else { Font::small() };
    let line_h = font.line_height();

    // 1. NovaOS Star Badge on far left
    let star_size = if rect.h >= 20 { 12 } else { 8 };
    let star_x = rect.x + 8;
    let star_y = rect.y + (rect.h - star_size) / 2;

    s.fill_rect(Rect::new(star_x + 3, star_y, star_size - 6, star_size), palette.accent_primary);
    s.fill_rect(Rect::new(star_x, star_y + 3, star_size, star_size - 6), palette.accent_primary);
    s.fill_rect(Rect::new(star_x + 2, star_y + 2, star_size - 4, star_size - 4), WHITE);

    // 2. Active Application Name in bold / distinct styling
    let app_sz = font.measure_str(menubar.app_name);
    let app_x = star_x + star_size + 8;
    let text_y = rect.y + (rect.h - line_h) / 2;
    draw_text_font(s, app_x, text_y, menubar.app_name, WHITE, &font);

    // Subtle vertical separator between App Name and Menus
    let sep_x = app_x + app_sz.w + 10;
    s.fill_rect(Rect::new(sep_x, rect.y + 4, 1, rect.h - 8), palette.border_subtle);

    // 3. Top-Level Menus
    let mut cur_x = sep_x + 8;
    for (i, m) in menubar.menus.iter().enumerate() {
        let sz = font.measure_str(m.title);
        let item_w = sz.w + 14;
        let item_rect = Rect::new(cur_x, rect.y + 2, item_w, rect.h - 4);

        if active_menu == Some(i) {
            draw_rounded_rect(s, item_rect, RADIUS_XS, palette.accent_primary);
            let tx = item_rect.x + (item_rect.w - sz.w) / 2;
            draw_text_font(s, tx, text_y, m.title, WHITE, &font);
        } else if hovered_menu == Some(i) {
            draw_rounded_rect(s, item_rect, RADIUS_XS, palette.surface_raised);
            let tx = item_rect.x + (item_rect.w - sz.w) / 2;
            draw_text_font(s, tx, text_y, m.title, palette.text_primary, &font);
        } else {
            let tx = item_rect.x + (item_rect.w - sz.w) / 2;
            draw_text_font(s, tx, text_y, m.title, palette.text_secondary, &font);
        }

        cur_x += item_w + 2;
    }

    // 4. Clock and System Indicators on far right
    let clock_sz = font.measure_str(clock_str);
    let clock_x = rect.right() - clock_sz.w - 12;
    draw_text_font(s, clock_x, text_y, clock_str, palette.text_primary, &font);

    // Small status indicator dot next to clock
    let dot_x = clock_x - 14;
    let dot_y = rect.y + (rect.h - 6) / 2;
    draw_rounded_rect(s, Rect::new(dot_x, dot_y, 6, 6), 3, palette.status_success);
}

/// Computes the bounding rectangle for an open dropdown menu.
pub fn measure_dropdown_rect(
    anchor_x: i32,
    anchor_y: i32,
    menu: &Menu,
    max_w: i32,
    font: &Font,
) -> Rect {
    let mut min_w = 140;
    for item in menu.items {
        if !item.is_separator {
            let l_sz = font.measure_str(item.label);
            let s_sz = font.measure_str(item.shortcut);
            let total = l_sz.w + s_sz.w + 36;
            if total > min_w {
                min_w = total;
            }
        }
    }

    let item_h = if font.line_height() > 10 { 22 } else { 14 };
    let total_h = menu.items.iter().map(|item| if item.is_separator { 6 } else { item_h }).sum::<i32>() + 8;

    let x = anchor_x.min(max_w - min_w - 4).max(0);
    Rect::new(x, anchor_y, min_w, total_h)
}

/// Renders an active dropdown menu overlay directly beneath an activated menu bar item.
pub fn draw_dropdown_surface<S: Surface + ?Sized>(
    s: &mut S,
    bounds: Rect,
    menu: &Menu,
    hovered_item: Option<usize>,
    palette: &ThemePalette,
) {
    if bounds.w <= 0 || bounds.h <= 0 {
        return;
    }

    let font = if bounds.h > 100 { Font::regular() } else { Font::small() };
    let item_h = if font.line_height() > 10 { 22 } else { 14 };

    // Drop shadow
    s.fill_rect(Rect::new(bounds.x + 3, bounds.bottom(), bounds.w - 1, 3), Color::rgba(0, 0, 0, 50));
    s.fill_rect(Rect::new(bounds.right(), bounds.y + 3, 3, bounds.h), Color::rgba(0, 0, 0, 50));

    // Panel background & focused outline border
    draw_rounded_rect(s, bounds, RADIUS_MD, palette.surface_raised);
    draw_rect_outline(s, bounds, 1, palette.border_focused);

    // Inner subtle highlight line
    s.fill_rect(Rect::new(bounds.x + 1, bounds.y + 1, bounds.w - 2, 1), palette.surface_raised.lighten(25));

    let mut cur_y = bounds.y + 4;
    for (idx, item) in menu.items.iter().enumerate() {
        if item.is_separator {
            s.fill_rect(Rect::new(bounds.x + 6, cur_y + 2, bounds.w - 12, 1), palette.border_subtle);
            cur_y += 6;
        } else {
            let item_rect = Rect::new(bounds.x + 4, cur_y, bounds.w - 8, item_h);
            let is_hovered = hovered_item == Some(idx) && item.is_enabled;

            if is_hovered {
                draw_rounded_rect(s, item_rect, RADIUS_XS, palette.accent_primary);
            }

            let text_color = if !item.is_enabled {
                palette.text_muted
            } else if is_hovered {
                WHITE
            } else {
                palette.text_primary
            };

            let shortcut_color = if is_hovered {
                palette.surface_sunken.lighten(180)
            } else {
                palette.text_secondary
            };

            let text_y = item_rect.y + (item_h - font.line_height()) / 2;
            draw_text_font(s, item_rect.x + 8, text_y, item.label, text_color, &font);

            if !item.shortcut.is_empty() {
                let sc_sz = font.measure_str(item.shortcut);
                let sc_x = item_rect.right() - sc_sz.w - 8;
                draw_text_font(s, sc_x, text_y, item.shortcut, shortcut_color, &font);
            }

            cur_y += item_h;
        }
    }
}
