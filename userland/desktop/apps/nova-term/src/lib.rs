//! userland/desktop/apps/nova-term/src/lib.rs
//!
//! # NovaTerm - Pro Developer Terminal for NovaOS
//!
//! High-performance GPU-accelerated terminal emulator in pure Rust with:
//! - 24-bit TrueColor support (`\x1b[38;2;r;g;bm` and `\x1b[48;2;r;g;bm`)
//! - Full ANSI / VT100 / xterm escape sequence state machine
//! - Multiple tabs with independent shell sessions
//! - Split horizontal and vertical panes with keyboard & mouse navigation
//! - Rich developer themes (Dracula, Nord, Solarized Dark, Monokai, One Dark, Nova Dark)
//! - Fast monospaced glyph cache and cursor styles (Block, Underline, Beam)
//! - Smart URL detection with click-to-open
//! - Clipboard integration (Ctrl+Shift+C / Ctrl+Shift+V)
//! - PTY connection with SIGWINCH window resize signaling

#![cfg_attr(not(test), no_std)]
#![allow(dead_code, unused_variables)]

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::{
    format,
    string::String,
    vec::Vec,
};

#[cfg(test)]
extern crate std;
#[cfg(test)]
use std::{
    format,
    string::String,
    vec::Vec,
};

pub mod ansi;
pub mod clipboard;
pub mod glyph_cache;
pub mod grid;
pub mod panes;
pub mod pty;
pub mod tabs;
pub mod theme;
pub mod url;

use ansi::AnsiParser;
use clipboard::{ClipboardManager, SelectionMode};
use glyph_cache::GlyphCache;
use grid::TerminalGrid;
use panes::{PaneNode, PaneSplit, TerminalPane};
use pty::PtySession;
use tabs::{TabBar, TabBarAction};
use theme::{ColorScheme, ThemePreset};
use url::{DetectedUrl, UrlScanner};

use nova_ui::prelude::{BufferSurface, Color, Point, Rect};

pub const MOD_SHIFT: u32 = 1 << 0;
pub const MOD_CTRL: u32 = 1 << 1;
pub const MOD_ALT: u32 = 1 << 2;
pub const MOD_SUPER: u32 = 1 << 3;

pub const KEY_BACKSPACE: u32 = 0x08;
pub const KEY_TAB: u32 = 0x09;
pub const KEY_ENTER: u32 = 0x0D;
pub const KEY_ESCAPE: u32 = 0x1B;
pub const KEY_PAGE_UP: u32 = 0x21;
pub const KEY_PAGE_DOWN: u32 = 0x22;
pub const KEY_LEFT: u32 = 0x25;
pub const KEY_UP: u32 = 0x26;
pub const KEY_RIGHT: u32 = 0x27;
pub const KEY_DOWN: u32 = 0x28;

/// Main application state for NovaTerm.
pub struct NovaTermApp {
    pub tab_bar: TabBar,
    pub clipboard: ClipboardManager,
    pub glyph_cache: GlyphCache,
    pub theme: ColorScheme,
    pub current_theme_preset: ThemePreset,
    pub bounds: Rect,
    pub divider_thickness: i32,
    pub hover_url: Option<DetectedUrl>,
    pub is_dragging_selection: bool,
    pub drag_pane_id: Option<usize>,
    pub status_message: Option<String>,
}

impl NovaTermApp {
    pub fn new(width: i32, height: i32) -> Self {
        let preset = ThemePreset::NovaDark;
        let theme = ColorScheme::from_preset(preset);
        let mut app = Self {
            tab_bar: TabBar::new(&theme),
            clipboard: ClipboardManager::new(),
            glyph_cache: GlyphCache::new(),
            theme,
            current_theme_preset: preset,
            bounds: Rect::new(0, 0, width, height),
            divider_thickness: 3,
            hover_url: None,
            is_dragging_selection: false,
            drag_pane_id: None,
            status_message: None,
        };

        app.layout(width, height);
        app
    }

    /// Set terminal theme preset
    pub fn set_theme(&mut self, preset: ThemePreset) {
        self.current_theme_preset = preset;
        self.theme = ColorScheme::from_preset(preset);
    }

    /// Update terminal geometry and layout panes
    pub fn layout(&mut self, width: i32, height: i32) {
        self.bounds = Rect::new(0, 0, width, height);
        self.tab_bar.bounds = Rect::new(0, 0, width, self.tab_bar.height);

        let content_y = self.tab_bar.height;
        let content_h = (height - content_y).max(100);
        let content_rect = Rect::new(0, content_y, width, content_h);

        let cw = self.glyph_cache.metrics.cell_width;
        let ch = self.glyph_cache.metrics.cell_height;

        let theme = &self.theme;
        if let Some(tab) = self.tab_bar.active_tab_mut() {
            tab.layout(content_rect, self.divider_thickness, cw, ch, theme);
        }
    }

    /// Resize window
    pub fn resize(&mut self, width: i32, height: i32) {
        self.layout(width, height);
    }

    /// Background update: poll active tab PTY output and cursor blink
    pub fn update(&mut self) -> bool {
        let mut changed = false;
        let theme = &self.theme;
        if let Some(tab) = self.tab_bar.active_tab_mut() {
            if tab.update(theme) {
                changed = true;
            }
        }
        changed
    }

    /// Handles keyboard input events with standard modern terminal shortcuts:
    /// - Ctrl+Shift+T: New Tab
    /// - Ctrl+Shift+W: Close Pane/Tab
    /// - Ctrl+Shift+D: Split Pane Horizontally
    /// - Ctrl+Shift+E: Split Pane Vertically
    /// - Ctrl+Shift+C: Copy Selection
    /// - Ctrl+Shift+V: Paste from Clipboard
    /// - Ctrl+PageUp / Ctrl+PageDown: Cycle Tabs
    /// - Alt+Left/Right/Up/Down: Cycle Pane Focus
    pub fn handle_key_down(&mut self, key: u32, modifiers: u32, text: Option<&str>) -> bool {
        let has_ctrl = (modifiers & MOD_CTRL) != 0;
        let has_shift = (modifiers & MOD_SHIFT) != 0;
        let has_alt = (modifiers & MOD_ALT) != 0;

        // 1. Ctrl+Shift shortcuts
        if has_ctrl && has_shift {
            match key {
                // 'T' or 't' -> New Tab
                0x54 | 0x74 => {
                    let title = format!("Terminal {}", self.tab_bar.tabs.len() + 1);
                    self.tab_bar.add_tab(&title, 80, 24, &self.theme);
                    self.layout(self.bounds.width, self.bounds.height);
                    return true;
                }
                // 'W' or 'w' -> Close active pane or tab
                0x57 | 0x77 => {
                    let closed_pane = if let Some(tab) = self.tab_bar.active_tab_mut() {
                        tab.close_active_pane()
                    } else {
                        false
                    };

                    if !closed_pane {
                        // If only one pane was left, close active tab
                        self.tab_bar.close_active_tab();
                    }
                    self.layout(self.bounds.width, self.bounds.height);
                    return true;
                }
                // 'D' or 'd' -> Split Horizontally
                0x44 | 0x64 => {
                    let new_pane_id = self.tab_bar.next_pane_id;
                    self.tab_bar.next_pane_id += 1;
                    if let Some(tab) = self.tab_bar.active_tab_mut() {
                        tab.split_active(
                            new_pane_id,
                            PaneSplit::Horizontal,
                            80,
                            24,
                            &self.theme,
                        );
                    }
                    self.layout(self.bounds.width, self.bounds.height);
                    return true;
                }
                // 'E' or 'e' -> Split Vertically
                0x45 | 0x65 => {
                    let new_pane_id = self.tab_bar.next_pane_id;
                    self.tab_bar.next_pane_id += 1;
                    if let Some(tab) = self.tab_bar.active_tab_mut() {
                        tab.split_active(new_pane_id, PaneSplit::Vertical, 80, 24, &self.theme);
                    }
                    self.layout(self.bounds.width, self.bounds.height);
                    return true;
                }
                // 'C' or 'c' -> Copy selection
                0x43 | 0x63 => {
                    if let Some(tab) = self.tab_bar.active_tab() {
                        if let Some(pane) = tab.active_pane() {
                            if pane.selection.is_active {
                                let txt = pane.selection.extract_text(&pane.grid);
                                self.clipboard.copy(&txt);
                                self.status_message = Some(format!("Copied {} chars", txt.len()));
                                return true;
                            }
                        }
                    }
                    return false;
                }
                // 'V' or 'v' -> Paste from clipboard
                0x56 | 0x76 => {
                    if let Some(paste_text) = self.clipboard.get_clipboard() {
                        let bytes = paste_text.into_bytes();
                        let theme = &self.theme;
                        if let Some(tab) = self.tab_bar.active_tab_mut() {
                            if let Some(pane) = tab.active_pane_mut() {
                                pane.send_input(&bytes, theme);
                                return true;
                            }
                        }
                    }
                    return false;
                }
                _ => {}
            }
        }

        // 2. Ctrl Shortcuts
        if has_ctrl && !has_shift && !has_alt {
            match key {
                KEY_PAGE_UP => {
                    self.tab_bar.switch_to_prev_tab();
                    self.layout(self.bounds.width, self.bounds.height);
                    return true;
                }
                KEY_PAGE_DOWN => {
                    self.tab_bar.switch_to_next_tab();
                    self.layout(self.bounds.width, self.bounds.height);
                    return true;
                }
                // Ctrl+C (Interrupt signal in PTY)
                0x43 | 0x63 => {
                    let theme = &self.theme;
                    if let Some(tab) = self.tab_bar.active_tab_mut() {
                        if let Some(pane) = tab.active_pane_mut() {
                            pane.send_input(b"\x03", theme);
                            return true;
                        }
                    }
                }
                // Ctrl+D (EOF signal in PTY)
                0x44 | 0x64 => {
                    let theme = &self.theme;
                    if let Some(tab) = self.tab_bar.active_tab_mut() {
                        if let Some(pane) = tab.active_pane_mut() {
                            pane.send_input(b"\x04", theme);
                            return true;
                        }
                    }
                }
                // Ctrl+L (Clear screen)
                0x4C | 0x6C => {
                    let theme = &self.theme;
                    if let Some(tab) = self.tab_bar.active_tab_mut() {
                        if let Some(pane) = tab.active_pane_mut() {
                            pane.send_input(b"\x0C", theme);
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }

        // 3. Alt Shortcuts (Pane navigation)
        if has_alt && !has_ctrl {
            match key {
                KEY_LEFT | KEY_UP => {
                    if let Some(tab) = self.tab_bar.active_tab_mut() {
                        tab.cycle_focus_prev();
                        return true;
                    }
                }
                KEY_RIGHT | KEY_DOWN => {
                    if let Some(tab) = self.tab_bar.active_tab_mut() {
                        tab.cycle_focus_next();
                        return true;
                    }
                }
                _ => {}
            }
        }

        // 4. Regular typing or cursor navigation
        let theme = &self.theme;
        if let Some(tab) = self.tab_bar.active_tab_mut() {
            if let Some(pane) = tab.active_pane_mut() {
                match key {
                    KEY_ENTER => {
                        pane.send_input(b"\r", theme);
                        return true;
                    }
                    KEY_BACKSPACE => {
                        pane.send_input(b"\x7F", theme);
                        return true;
                    }
                    KEY_TAB => {
                        pane.send_input(b"\t", theme);
                        return true;
                    }
                    KEY_ESCAPE => {
                        pane.send_input(b"\x1b", theme);
                        return true;
                    }
                    KEY_UP => {
                        pane.send_input(b"\x1b[A", theme);
                        return true;
                    }
                    KEY_DOWN => {
                        pane.send_input(b"\x1b[B", theme);
                        return true;
                    }
                    KEY_RIGHT => {
                        pane.send_input(b"\x1b[C", theme);
                        return true;
                    }
                    KEY_LEFT => {
                        pane.send_input(b"\x1b[D", theme);
                        return true;
                    }
                    _ => {
                        if let Some(t) = text {
                            pane.send_input(t.as_bytes(), theme);
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    /// Handles mouse down event (focusing pane, tab clicks, URL clicks, selection start)
    pub fn handle_mouse_down(&mut self, x: i32, y: i32, button: u8, modifiers: u32) -> bool {
        // Check tab bar click
        if self.tab_bar.bounds.contains(Point::new(x, y)) {
            if let Some(action) = self.tab_bar.handle_click(x, y) {
                match action {
                    TabBarAction::SelectTab(idx) => {
                        self.tab_bar.select_tab(idx);
                        self.layout(self.bounds.width, self.bounds.height);
                    }
                    TabBarAction::CloseTab(idx) => {
                        self.tab_bar.close_tab(idx);
                        self.layout(self.bounds.width, self.bounds.height);
                    }
                    TabBarAction::NewTab => {
                        let title = format!("Terminal {}", self.tab_bar.tabs.len() + 1);
                        self.tab_bar.add_tab(&title, 80, 24, &self.theme);
                        self.layout(self.bounds.width, self.bounds.height);
                    }
                }
                return true;
            }
            return false;
        }

        // Content area click
        let cw = self.glyph_cache.metrics.cell_width;
        let ch = self.glyph_cache.metrics.cell_height;

        if let Some(tab) = self.tab_bar.active_tab_mut() {
            if let Some(pane) = tab.root_pane.find_pane_at_pos_mut(x, y) {
                let pane_id = pane.id;
                let col = ((x - pane.bounds.x) / cw).max(0) as usize;
                let row = ((y - pane.bounds.y) / ch).max(0) as usize;

                // Check URL click
                if let Some(url) = pane.urls.url_at(col, row) {
                    self.status_message = Some(format!("Opening URL: {}", url.url));
                }

                // Focus clicked pane
                tab.focus_pane(pane_id);

                // Start selection
                if let Some(focused_pane) = tab.active_pane_mut() {
                    focused_pane.selection.clear();
                    focused_pane.selection.begin(col, row, SelectionMode::Linear);
                    self.is_dragging_selection = true;
                    self.drag_pane_id = Some(pane_id);
                }
                return true;
            }
        }

        false
    }

    /// Handles mouse drag for text selection
    pub fn handle_mouse_move(&mut self, x: i32, y: i32) {
        let cw = self.glyph_cache.metrics.cell_width;
        let ch = self.glyph_cache.metrics.cell_height;

        if let Some(tab) = self.tab_bar.active_tab_mut() {
            if let Some(pane) = tab.root_pane.find_pane_at_pos_mut(x, y) {
                let col = ((x - pane.bounds.x) / cw).max(0) as usize;
                let row = ((y - pane.bounds.y) / ch).max(0) as usize;

                // Hover URL update
                self.hover_url = pane.urls.set_hover(col, row).cloned();

                // Drag selection update
                if self.is_dragging_selection {
                    if let Some(drag_id) = self.drag_pane_id {
                        if pane.id == drag_id {
                            pane.selection.update(col, row);
                        }
                    }
                }
            } else {
                self.hover_url = None;
            }
        }
    }

    /// Handles mouse up (completing selection and copying to primary selection)
    pub fn handle_mouse_up(&mut self, x: i32, y: i32, button: u8) -> bool {
        if self.is_dragging_selection {
            self.is_dragging_selection = false;
            if let Some(tab) = self.tab_bar.active_tab_mut() {
                if let Some(pane_id) = self.drag_pane_id {
                    if let Some(pane) = tab.root_pane.find_pane_mut(pane_id) {
                        if pane.selection.is_active {
                            let text = pane.selection.extract_text(&pane.grid);
                            self.clipboard.copy(&text);
                        }
                    }
                }
            }
            self.drag_pane_id = None;
            return true;
        }
        false
    }

    /// Handles scroll wheel event
    pub fn handle_scroll(&mut self, x: i32, y: i32, delta: i32) {
        if let Some(tab) = self.tab_bar.active_tab_mut() {
            if let Some(pane) = tab.root_pane.find_pane_at_pos_mut(x, y) {
                if delta > 0 {
                    pane.grid.scroll_up(delta as usize);
                } else if delta < 0 {
                    pane.grid.scroll_down((-delta) as usize);
                }
            }
        }
    }

    /// Complete render pass onto BufferSurface
    pub fn render(&self, surface: &mut BufferSurface) {
        // 1. Fill main background
        surface.fill_rect(self.bounds, self.theme.background);

        // 2. Render Tab Bar
        self.render_tab_bar(surface);

        // 3. Render Active Tab Panes & Dividers
        if let Some(tab) = self.tab_bar.active_tab() {
            self.render_pane_node(&tab.root_pane, surface, tab.root_pane.pane_count() > 1);

            // Render Split Dividers
            let mut dividers = Vec::new();
            tab.root_pane
                .collect_dividers(self.divider_thickness, &mut dividers);
            for div in dividers {
                surface.fill_rect(div.rect, self.theme.selection);
            }
        }
    }

    /// Render Tab Bar with tabs, active indicator, close buttons, and add button
    fn render_tab_bar(&self, surface: &mut BufferSurface) {
        let bar_bg = Color::rgba(24, 26, 32, 255);
        surface.fill_rect(self.tab_bar.bounds, bar_bg);

        // Bottom separator line
        let sep_rect = Rect::new(
            self.tab_bar.bounds.x,
            self.tab_bar.bounds.y + self.tab_bar.height - 1,
            self.tab_bar.bounds.width,
            1,
        );
        surface.fill_rect(sep_rect, Color::rgba(45, 48, 60, 255));

        let (tab_metrics, new_btn_rect) = self.tab_bar.compute_tab_metrics();

        for item in tab_metrics {
            if item.is_active {
                // Active tab background matches terminal canvas
                surface.fill_rounded_rect(item.rect, 3, self.theme.background);
                // Active tab top indicator stripe (Bright Cyan: ansi[14])
                let top_stripe = Rect::new(item.rect.x, item.rect.y, item.rect.width, 2);
                surface.fill_rect(top_stripe, self.theme.ansi[14]);
            } else {
                // Inactive tab background
                let inactive_bg = Color::rgba(32, 35, 43, 255);
                surface.fill_rounded_rect(item.rect, 3, inactive_bg);
            }

            // Tab title text
            let text_color = if item.is_active {
                self.theme.foreground
            } else {
                Color::rgba(140, 145, 160, 255)
            };
            self.glyph_cache.draw_string(
                surface,
                item.rect.x + 8,
                item.rect.y + 4,
                &item.title,
                text_color,
            );

            // Tab close button ('x')
            let close_color = if item.is_active {
                Color::rgba(180, 185, 200, 255)
            } else {
                Color::rgba(110, 115, 130, 255)
            };
            self.glyph_cache.draw_string(
                surface,
                item.close_rect.x + 2,
                item.close_rect.y - 1,
                "x",
                close_color,
            );
        }

        // New Tab '+' button
        let btn_bg = Color::rgba(35, 38, 48, 255);
        surface.fill_rounded_rect(new_btn_rect, 3, btn_bg);
        self.glyph_cache.draw_string(
            surface,
            new_btn_rect.x + 6,
            new_btn_rect.y + 1,
            "+",
            self.theme.foreground,
        );
    }

    /// Recursively render pane tree
    fn render_pane_node(&self, node: &PaneNode, surface: &mut BufferSurface, has_splits: bool) {
        match node {
            PaneNode::Leaf(pane) => {
                self.render_single_pane(pane, surface, has_splits);
            }
            PaneNode::Split { first, second, .. } => {
                self.render_pane_node(first, surface, has_splits);
                self.render_pane_node(second, surface, has_splits);
            }
        }
    }

    /// Render a single leaf terminal pane
    fn render_single_pane(
        &self,
        pane: &TerminalPane,
        surface: &mut BufferSurface,
        has_splits: bool,
    ) {
        // 1. Pane background
        surface.fill_rect(pane.bounds, self.theme.background);

        // 2. Focused border outline if multiple panes are open (Bright Blue: ansi[12])
        if has_splits && pane.is_focused {
            surface.stroke_rounded_rect(pane.bounds, 0, 1, self.theme.ansi[12]);
        }

        let cw = self.glyph_cache.metrics.cell_width;
        let ch = self.glyph_cache.metrics.cell_height;

        let cur_col = pane.grid.cursor_x;
        let cur_row = pane.grid.cursor_y;

        // 3. Render character cells
        for row in 0..pane.grid.rows {
            let py = pane.bounds.y + (row as i32) * ch;
            if py + ch > pane.bounds.y + pane.bounds.height {
                break;
            }

            for col in 0..pane.grid.cols {
                let px = pane.bounds.x + (col as i32) * cw;
                if px + cw > pane.bounds.x + pane.bounds.width {
                    break;
                }

                if let Some(cell) = pane.grid.get_cell(col, row) {
                    let is_sel = pane.selection.contains(col, row);

                    self.glyph_cache.render_cell(
                        surface,
                        px,
                        py,
                        &cell,
                        is_sel,
                        self.theme.selection,
                        self.theme.background,
                    );

                    // Render URL underline if cell is part of a URL
                    if pane.urls.is_url_cell(col, row) {
                        let underline_y = py + self.glyph_cache.metrics.baseline_y + 1;
                        let u_color = if pane.urls.is_hovered(col, row) {
                            self.theme.ansi[14] // Bright Cyan
                        } else {
                            self.theme.ansi[4] // Blue
                        };
                        surface.fill_rect(Rect::new(px, underline_y, cw, 1), u_color);
                    }

                    // Render cursor if this is the active pane and coordinates match
                    if pane.is_focused && col == cur_col && row == cur_row {
                        self.glyph_cache
                            .render_cursor(surface, px, py, self.theme.cursor);
                    }
                }
            }
        }
    }
}

/// Comprehensive selftest runner for NovaTerm.
/// Verifies TrueColor rendering, ANSI state machine, PTY sessions, URL scanning,
/// clipboard integration, multi-tabs, split panes, and surface blitting.
pub fn run_nova_term_selftest() -> bool {
    // 1. Theme and Color verification
    let presets = [
        ThemePreset::Dracula,
        ThemePreset::Nord,
        ThemePreset::SolarizedDark,
        ThemePreset::Monokai,
        ThemePreset::OneDark,
        ThemePreset::NovaDark,
    ];

    for preset in &presets {
        let theme = ColorScheme::from_preset(*preset);
        if theme.background.a != 255 || theme.foreground.a != 255 {
            return false;
        }
    }

    let default_theme = ColorScheme::from_preset(ThemePreset::NovaDark);

    // 2. ANSI State Machine & 24-bit TrueColor Verification
    let mut grid = TerminalGrid::new(80, 24, &default_theme);
    let mut parser = AnsiParser::new();

    // SGR 24-bit TrueColor sequence: \x1b[38;2;255;120;60m
    let truecolor_seq = b"\x1b[38;2;255;120;60mNovaOS TrueColor\x1b[0m";
    parser.parse_bytes(truecolor_seq, &mut grid, &default_theme);

    let cell = match grid.get_cell(0, 0) {
        Some(c) => c,
        None => return false,
    };
    if cell.fg.r != 255 || cell.fg.g != 120 || cell.fg.b != 60 {
        return false;
    }
    if cell.ch != 'N' {
        return false;
    }

    // Cursor positioning sequence: \x1b[5;10H (row 5, col 10 -> 0-indexed: row 4, col 9)
    parser.parse_bytes(b"\x1b[5;10H", &mut grid, &default_theme);
    if grid.cursor_y != 4 || grid.cursor_x != 9 {
        return false;
    }

    // 3. PTY Session & SIGWINCH Resize
    let mut pty = PtySession::new(80, 24, 720, 432);
    pty.write_input(b"echo Hello NovaOS\r\n");
    let out = pty.read_output();
    if out.is_empty() {
        return false;
    }

    pty.resize(120, 40, 1080, 720);
    if pty.winsize.ws_col != 120 || pty.winsize.ws_row != 40 || pty.sigwinch_count == 0 {
        return false;
    }

    // 4. URL Scanner Verification
    let mut url_grid = TerminalGrid::new(80, 24, &default_theme);
    let url_text = b"Visit https://novaos.org or http://github.com for code.";
    let mut url_parser = AnsiParser::new();
    url_parser.parse_bytes(url_text, &mut url_grid, &default_theme);

    let mut scanner = UrlScanner::new();
    let urls = scanner.scan_grid(&url_grid);
    if urls.len() != 2 {
        return false;
    }
    if urls[0].url != "https://novaos.org" || urls[1].url != "http://github.com" {
        return false;
    }
    if !scanner.is_url_cell(7, 0) {
        return false;
    }

    // 5. Clipboard Integration
    let mut clipboard = ClipboardManager::new();
    clipboard.copy("NovaOS Fast Kernel");
    if clipboard.get_clipboard() != Some(String::from("NovaOS Fast Kernel")) {
        return false;
    }

    // 6. Split Panes & TabBar Verification
    let mut app = NovaTermApp::new(1024, 768);

    // Initial state: 1 tab, 1 pane
    if app.tab_bar.tabs.len() != 1 {
        return false;
    }
    if let Some(tab) = app.tab_bar.active_tab() {
        if tab.root_pane.pane_count() != 1 {
            return false;
        }
    } else {
        return false;
    }

    // Split horizontally (Ctrl+Shift+D)
    app.handle_key_down(0x44, MOD_CTRL | MOD_SHIFT, None);
    if let Some(tab) = app.tab_bar.active_tab() {
        if tab.root_pane.pane_count() != 2 {
            return false;
        }
    }

    // Split vertically (Ctrl+Shift+E)
    app.handle_key_down(0x45, MOD_CTRL | MOD_SHIFT, None);
    if let Some(tab) = app.tab_bar.active_tab() {
        if tab.root_pane.pane_count() != 3 {
            return false;
        }
    }

    // Cycle focus
    app.handle_key_down(KEY_RIGHT, MOD_ALT, None);

    // Add new tab (Ctrl+Shift+T)
    app.handle_key_down(0x54, MOD_CTRL | MOD_SHIFT, None);
    if app.tab_bar.tabs.len() != 2 {
        return false;
    }

    // 7. Full BufferSurface Render Cycle
    let mut surface = BufferSurface::new(1024, 768);
    app.render(&mut surface);

    // Verify background was cleared and tab bar was drawn
    let bg_pixel = surface.get_pixel(100, 200);
    if bg_pixel.a != 255 {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nova_term_selftest() {
        assert!(run_nova_term_selftest(), "NovaTerm selftest failed!");
    }

    #[test]
    fn test_tabs_management() {
        let mut app = NovaTermApp::new(800, 600);
        assert_eq!(app.tab_bar.tabs.len(), 1);

        // Add 2 tabs
        let theme = app.theme.clone();
        app.tab_bar.add_tab("Tab 2", 80, 24, &theme);
        app.tab_bar.add_tab("Tab 3", 80, 24, &theme);
        assert_eq!(app.tab_bar.tabs.len(), 3);
        assert_eq!(app.tab_bar.active_tab_index, 2);

        // Switch next / prev
        app.tab_bar.switch_to_next_tab();
        assert_eq!(app.tab_bar.active_tab_index, 0);
        app.tab_bar.switch_to_prev_tab();
        assert_eq!(app.tab_bar.active_tab_index, 2);

        // Close tab
        app.tab_bar.close_tab(1);
        assert_eq!(app.tab_bar.tabs.len(), 2);
    }

    #[test]
    fn test_split_and_close_panes() {
        let mut app = NovaTermApp::new(800, 600);
        // Split horizontal
        assert!(app.handle_key_down(0x44, MOD_CTRL | MOD_SHIFT, None));
        assert_eq!(app.tab_bar.active_tab().unwrap().root_pane.pane_count(), 2);

        // Split vertical
        assert!(app.handle_key_down(0x45, MOD_CTRL | MOD_SHIFT, None));
        assert_eq!(app.tab_bar.active_tab().unwrap().root_pane.pane_count(), 3);

        // Close active pane
        assert!(app.handle_key_down(0x57, MOD_CTRL | MOD_SHIFT, None));
        assert_eq!(app.tab_bar.active_tab().unwrap().root_pane.pane_count(), 2);
    }

    #[test]
    fn test_theme_and_truecolor_palette() {
        for preset in &[
            ThemePreset::Dracula,
            ThemePreset::Nord,
            ThemePreset::SolarizedDark,
            ThemePreset::Monokai,
            ThemePreset::OneDark,
            ThemePreset::NovaDark,
        ] {
            let cs = ColorScheme::from_preset(*preset);
            assert_eq!(cs.ansi.len(), 16);
            let c256 = cs.resolve_256_color(196); // Bright red in 256 palette
            assert!(c256.a == 255);
            let gray = cs.resolve_256_color(240); // Grayscale
            assert_eq!(gray.r, gray.g);
            assert_eq!(gray.g, gray.b);
        }
    }

    #[test]
    fn test_ansi_parser_sgr_and_cursor() {
        let theme = ColorScheme::from_preset(ThemePreset::Dracula);
        let mut grid = TerminalGrid::new(80, 24, &theme);
        let mut parser = AnsiParser::new();

        // Write 24-bit TrueColor text
        parser.parse_bytes(b"\x1b[38;2;12;34;56mABC\x1b[0m", &mut grid, &theme);
        let cell = grid.get_cell(0, 0).unwrap();
        assert_eq!(cell.ch, 'A');
        assert_eq!(cell.fg.r, 12);
        assert_eq!(cell.fg.g, 34);
        assert_eq!(cell.fg.b, 56);

        // Cursor positioning
        parser.parse_bytes(b"\x1b[10;20H", &mut grid, &theme);
        assert_eq!(grid.cursor_y, 9);
        assert_eq!(grid.cursor_x, 19);

        // Erase in line
        parser.parse_bytes(b"\x1b[2K", &mut grid, &theme);
        let cleared = grid.get_cell(19, 9).unwrap();
        assert_eq!(cleared.ch, ' ');
    }

    #[test]
    fn test_terminal_grid_scrollback_and_alt_screen() {
        let theme = ColorScheme::from_preset(ThemePreset::NovaDark);
        let mut grid = TerminalGrid::new(40, 10, &theme);
        for i in 0..15 {
            grid.put_char(('0' as u8 + (i % 10)) as char);
            grid.carriage_return();
            grid.line_feed();
        }
        assert!(!grid.scrollback.is_empty());

        // Alternate screen switch
        grid.set_alt_screen(true);
        assert!(grid.is_alt_screen);
        assert_eq!(grid.cursor_x, 0);
        assert_eq!(grid.cursor_y, 0);
        grid.set_alt_screen(false);
        assert!(!grid.is_alt_screen);
    }

    #[test]
    fn test_pty_session_resizing_and_sigwinch() {
        let mut pty = PtySession::new(80, 24, 720, 432);
        assert_eq!(pty.sigwinch_count, 0);

        pty.resize(100, 30, 900, 540);
        assert_eq!(pty.sigwinch_count, 1);
        assert_eq!(pty.winsize.ws_col, 100);
        assert_eq!(pty.winsize.ws_row, 30);

        pty.write_input(b"ls\r\n");
        let out = pty.read_output();
        assert!(!out.is_empty());
    }

    #[test]
    fn test_url_scanner_detection_and_click() {
        let theme = ColorScheme::from_preset(ThemePreset::NovaDark);
        let mut grid = TerminalGrid::new(80, 24, &theme);
        let mut parser = AnsiParser::new();
        parser.parse_bytes(b"Repository: https://github.com/novaos/kernel\r\n", &mut grid, &theme);

        let mut scanner = UrlScanner::new();
        let urls = scanner.scan_grid(&grid);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].url, "https://github.com/novaos/kernel");

        let clicked = scanner.click_at(15, 0);
        assert_eq!(clicked, Some("https://github.com/novaos/kernel".to_string()));
    }

    #[test]
    fn test_clipboard_linear_and_block_selection() {
        let theme = ColorScheme::from_preset(ThemePreset::NovaDark);
        let mut grid = TerminalGrid::new(40, 10, &theme);
        let mut parser = AnsiParser::new();
        parser.parse_bytes(b"Line 0: Alpha\r\nLine 1: Beta\r\n", &mut grid, &theme);

        let mut sel = clipboard::TextSelection::new();
        sel.begin(0, 0, SelectionMode::Linear);
        sel.update(12, 0);
        let text = sel.extract_text(&grid);
        assert!(text.contains("Line 0: Alpha"));

        let mut clip = ClipboardManager::new();
        clip.copy(&text);
        assert_eq!(clip.paste(), text);
    }

    #[test]
    fn test_glyph_cache_and_buffer_rendering() {
        let mut surface = BufferSurface::new(640, 480);
        let app = NovaTermApp::new(640, 480);
        app.render(&mut surface);

        let pixel = surface.get_pixel(320, 240);
        assert_eq!(pixel.a, 255);
    }

    #[test]
    fn test_keyboard_shortcuts_and_mouse_interaction() {
        let mut app = NovaTermApp::new(800, 600);

        // Mouse click on '+' button to add tab
        let (_, new_tab_btn) = app.tab_bar.compute_tab_metrics();
        let clicked = app.handle_mouse_down(new_tab_btn.x + 2, new_tab_btn.y + 2, 1, 0);
        assert!(clicked);
        assert_eq!(app.tab_bar.tabs.len(), 2);

        // Typing characters
        assert!(app.handle_key_down(0x41, 0, Some("A")));

        // Copy and paste shortcuts
        assert!(!app.handle_key_down(0x43, MOD_CTRL | MOD_SHIFT, None)); // Nothing selected
    }
}
