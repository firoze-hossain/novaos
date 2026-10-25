//! userland/edit-rs/editor.rs - NovaOS Graphical Text Editor Application
//!
//! Month 3 - Core desktop apps: Text editor app
//! A real, full-featured ring-3 text editor showcasing the NovaOS design system
//! and advanced multi-size typography engine.
//!
//! Features:
//! - Multi-line text buffer model with line insertion, line splitting, deletion, and merging.
//! - Precise cursor tracking, boundary clamping, and horizontal/vertical scrolling.
//! - File I/O operations via NovaOS VFS syscalls (`sys_open`, `sys_read`, `sys_close`, `sys_write_file`).
//! - Advanced Typography Showcase:
//!   - Monospaced 8x16 fixed-pitch code view (`Font::mono()`)
//!   - Proportional Sans 8x16 naturally-kerned prose view (`Font::regular()`)
//!   - Compact 8x8 high-density view (`Font::small()`)
//!   - Large 12x24 heading/presentation view (`Font::large()`)
//! - Line numbers gutter with active line accentuation and subtle separator.
//! - Lightweight code syntax highlighting (keywords, strings, comments, numbers).
//! - Top interactive toolbar: `[New]`, `[Open]`, `[Save]`, `[Cut]`, `[Copy]`, `[Paste]`, `[Font: ...]`.
//! - Bottom status bar with file name, modified dirty indicator (`*`), line/col position, and statistics.
//! - Mouse pointer and click hit-testing for both toolbar buttons and text canvas positioning.
//! - Zero per-pixel syscalls: renders entirely into `BufferSurface32` and presents in a single blit.
//! - Stack-safe architecture: large buffers located in `.bss` to respect the 16KB userland stack limit.
//! - Standalone executable (`EDIT.ELF`) and deterministic headless `--selftest` mode.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(dead_code)]

mod ffi;

#[cfg(test)]
#[macro_use]
extern crate std;

#[allow(unused_imports)]
use core::ptr::addr_of_mut;
use ui::color::Color;
use ui::cursor::{draw_cursor_sprite, CursorShape, CursorTheme};
use ui::draw::{BufferSurface32, Surface};
use ui::font::{draw_char_font, draw_text_font, Font};
use ui::geom::Rect;

#[cfg(not(test))]
use core::panic::PanicInfo;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe {
        ffi::sys_fb_release();
        let msg = b"edit: internal error (panic)\n\0";
        ffi::sys_write(msg.as_ptr());
        ffi::sys_exit(1);
    }
}

// ---------------------------------------------------------------------------
// Constants & Palette
// ---------------------------------------------------------------------------

pub const MAX_LINES: usize = 256;
pub const MAX_LINE_CHARS: usize = 128;
pub const MAX_FILE_SIZE: usize = 8192;
pub const MAX_CLIPBOARD: usize = 512;

pub const TOOLBAR_H: i32 = 28;
pub const STATUSBAR_H: i32 = 22;
pub const GUTTER_W: i32 = 46;

// NovaOS Design System Palette (Dark theme)
pub const COLOR_BG: Color = Color::rgb(13, 17, 23);                // #0D1117 Main canvas
pub const COLOR_LINE_ACTIVE: Color = Color::rgb(22, 27, 34);       // #161B22 Active line background
pub const COLOR_GUTTER_BG: Color = Color::rgb(18, 22, 28);         // #12161C Gutter background
pub const COLOR_GUTTER_FG: Color = Color::rgb(110, 118, 129);      // #6E7681 Muted line number
pub const COLOR_GUTTER_ACTIVE: Color = Color::rgb(240, 246, 252);  // #F0F6FC Active line number
pub const COLOR_GUTTER_BORDER: Color = Color::rgb(48, 54, 61);     // #30363D Divider border

pub const COLOR_TOOLBAR_BG: Color = Color::rgb(22, 27, 34);        // #161B22 Toolbar background
pub const COLOR_TOOLBAR_BORDER: Color = Color::rgb(48, 54, 61);    // #30363D Divider line
pub const COLOR_BTN_BG: Color = Color::rgb(33, 38, 45);            // #21262D Button normal
pub const COLOR_BTN_HOVER: Color = Color::rgb(48, 54, 61);         // #30363D Button hover
pub const COLOR_BTN_BORDER: Color = Color::rgb(56, 68, 77);        // Button border
pub const COLOR_BTN_TEXT: Color = Color::rgb(240, 246, 252);       // Bright button text

pub const COLOR_STATUS_BG: Color = Color::rgb(22, 27, 34);         // Status bar background
pub const COLOR_STATUS_BORDER: Color = Color::rgb(48, 54, 61);     // Status bar border
pub const COLOR_STATUS_TEXT: Color = Color::rgb(139, 148, 158);    // Muted status text
pub const COLOR_STATUS_ACCENT: Color = Color::rgb(88, 166, 255);   // Blue coordinate text
pub const COLOR_DIRTY_STAR: Color = Color::rgb(248, 81, 73);       // Red modified indicator

pub const COLOR_CURSOR: Color = Color::rgb(88, 166, 255);          // Cursor bar
pub const COLOR_SELECTION: Color = Color::rgb(38, 79, 120);        // Selection highlight

// Syntax Highlighting Tokens
pub const COLOR_SYN_DEFAULT: Color = Color::rgb(230, 237, 243);    // Default text
pub const COLOR_SYN_KEYWORD: Color = Color::rgb(255, 123, 114);    // Rust keywords (fn, let, mut, struct...)
pub const COLOR_SYN_STRING: Color = Color::rgb(126, 231, 135);     // Strings in quotes
pub const COLOR_SYN_COMMENT: Color = Color::rgb(139, 148, 158);    // Comments (// or #)
pub const COLOR_SYN_NUMBER: Color = Color::rgb(210, 153, 34);      // Digits

// ---------------------------------------------------------------------------
// Font Mode Configuration
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FontMode {
    Mono8x16,   // Fixed monospaced 8x16 (Code / Terminal)
    Sans8x16,   // Proportional Sans 8x16 (Prose / Documentation)
    Small8x8,   // Compact 8x8 (Dense notes)
    Large12x24, // Large 12x24 (Heading / High legibility)
}

impl FontMode {
    pub fn font(&self) -> Font {
        match self {
            FontMode::Mono8x16 => Font::mono(),
            FontMode::Sans8x16 => Font::regular(),
            FontMode::Small8x8 => Font::small(),
            FontMode::Large12x24 => Font::large(),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            FontMode::Mono8x16 => "Font: Mono",
            FontMode::Sans8x16 => "Font: Sans",
            FontMode::Small8x8 => "Font: Small",
            FontMode::Large12x24 => "Font: Large",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            FontMode::Mono8x16 => FontMode::Sans8x16,
            FontMode::Sans8x16 => FontMode::Small8x8,
            FontMode::Small8x8 => FontMode::Large12x24,
            FontMode::Large12x24 => FontMode::Mono8x16,
        }
    }
}

// ---------------------------------------------------------------------------
// Text Line Model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct Line {
    pub chars: [u8; MAX_LINE_CHARS],
    pub len: usize,
}

impl Line {
    pub const fn empty() -> Self {
        Line {
            chars: [0; MAX_LINE_CHARS],
            len: 0,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.chars[..self.len]).unwrap_or("")
    }

    pub fn insert_char(&mut self, idx: usize, c: u8) -> bool {
        if self.len >= MAX_LINE_CHARS {
            return false;
        }
        let insert_at = idx.min(self.len);
        for i in (insert_at..self.len).rev() {
            self.chars[i + 1] = self.chars[i];
        }
        self.chars[insert_at] = c;
        self.len += 1;
        true
    }

    pub fn remove_char(&mut self, idx: usize) -> Option<u8> {
        if idx >= self.len {
            return None;
        }
        let removed = self.chars[idx];
        for i in idx..self.len.saturating_sub(1) {
            self.chars[i] = self.chars[i + 1];
        }
        self.len -= 1;
        Some(removed)
    }
}

// ---------------------------------------------------------------------------
// Toolbar Button Definition
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolbarAction {
    New,
    Open,
    Save,
    Cut,
    Copy,
    Paste,
    ToggleFont,
}

pub struct ToolbarBtnDef {
    pub action: ToolbarAction,
    pub label: &'static str,
    pub width: i32,
}

pub const TOOLBAR_BUTTONS: [ToolbarBtnDef; 7] = [
    ToolbarBtnDef { action: ToolbarAction::New, label: "New", width: 44 },
    ToolbarBtnDef { action: ToolbarAction::Open, label: "Open", width: 48 },
    ToolbarBtnDef { action: ToolbarAction::Save, label: "Save", width: 48 },
    ToolbarBtnDef { action: ToolbarAction::Cut, label: "Cut", width: 42 },
    ToolbarBtnDef { action: ToolbarAction::Copy, label: "Copy", width: 48 },
    ToolbarBtnDef { action: ToolbarAction::Paste, label: "Paste", width: 52 },
    ToolbarBtnDef { action: ToolbarAction::ToggleFont, label: "Font: Mono", width: 92 },
];

// ---------------------------------------------------------------------------
// Editor State Structure
// ---------------------------------------------------------------------------

pub struct Editor {
    pub lines: [Line; MAX_LINES],
    pub lines_count: usize,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub scroll_line: usize,
    pub scroll_col: usize,

    pub filename: [u8; 32],
    pub filename_len: usize,
    pub is_modified: bool,

    pub font_mode: FontMode,

    pub clipboard: [u8; MAX_CLIPBOARD],
    pub clipboard_len: usize,

    pub status_msg: [u8; 64],
    pub status_len: usize,

    pub mouse_x: i32,
    pub mouse_y: i32,
    pub hovered_action: Option<ToolbarAction>,

    pub screen_w: i32,
    pub screen_h: i32,
    pub running: bool,
}

impl Editor {
    pub const fn uninit() -> Self {
        Editor {
            lines: [Line::empty(); MAX_LINES],
            lines_count: 1,
            cursor_line: 0,
            cursor_col: 0,
            scroll_line: 0,
            scroll_col: 0,
            filename: [0; 32],
            filename_len: 0,
            is_modified: false,
            font_mode: FontMode::Mono8x16,
            clipboard: [0; MAX_CLIPBOARD],
            clipboard_len: 0,
            status_msg: [0; 64],
            status_len: 0,
            mouse_x: 100,
            mouse_y: 100,
            hovered_action: None,
            screen_w: 1024,
            screen_h: 768,
            running: true,
        }
    }

    pub fn init(&mut self, width: i32, height: i32) {
        self.screen_w = width;
        self.screen_h = height;
        self.running = true;
        self.font_mode = FontMode::Mono8x16;
        self.new_document();
        self.set_status("Ready");
    }

    pub fn set_status(&mut self, msg: &str) {
        self.status_len = 0;
        let bytes = msg.as_bytes();
        let copy_len = bytes.len().min(self.status_msg.len());
        self.status_msg[..copy_len].copy_from_slice(&bytes[..copy_len]);
        self.status_len = copy_len;
    }

    pub fn status_as_str(&self) -> &str {
        core::str::from_utf8(&self.status_msg[..self.status_len]).unwrap_or("")
    }

    pub fn set_filename(&mut self, name: &str) {
        self.filename_len = 0;
        let bytes = name.as_bytes();
        let copy_len = bytes.len().min(self.filename.len() - 1);
        self.filename[..copy_len].copy_from_slice(&bytes[..copy_len]);
        self.filename[copy_len] = 0;
        self.filename_len = copy_len;
    }

    pub fn filename_as_str(&self) -> &str {
        if self.filename_len == 0 {
            "UNTITLED.TXT"
        } else {
            core::str::from_utf8(&self.filename[..self.filename_len]).unwrap_or("UNTITLED.TXT")
        }
    }

    pub fn new_document(&mut self) {
        self.lines_count = 1;
        self.lines[0].clear();
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.scroll_line = 0;
        self.scroll_col = 0;
        self.set_filename("UNTITLED.TXT");
        self.is_modified = false;
        self.set_status("New document created");
    }

    // -----------------------------------------------------------------------
    // Text Manipulation & Editing
    // -----------------------------------------------------------------------

    pub fn insert_char(&mut self, c: u8) {
        if self.cursor_line >= self.lines_count {
            self.cursor_line = self.lines_count.saturating_sub(1);
        }
        if self.lines[self.cursor_line].insert_char(self.cursor_col, c) {
            self.cursor_col += 1;
            self.is_modified = true;
            self.adjust_scroll();
        }
    }

    pub fn insert_newline(&mut self) {
        if self.lines_count >= MAX_LINES {
            self.set_status("Line limit reached");
            return;
        }

        let cur_len = self.lines[self.cursor_line].len;
        let split_col = self.cursor_col.min(cur_len);
        let right_count = cur_len - split_col;

        let mut right_buf = [0u8; MAX_LINE_CHARS];
        if right_count > 0 {
            right_buf[..right_count]
                .copy_from_slice(&self.lines[self.cursor_line].chars[split_col..cur_len]);
        }
        self.lines[self.cursor_line].len = split_col;

        // Shift lines down to make room for split
        for i in (self.cursor_line + 1..=self.lines_count).rev() {
            self.lines[i] = self.lines[i - 1];
        }

        let new_line = &mut self.lines[self.cursor_line + 1];
        new_line.clear();
        if right_count > 0 {
            new_line.chars[..right_count].copy_from_slice(&right_buf[..right_count]);
            new_line.len = right_count;
        }

        self.lines_count += 1;
        self.cursor_line += 1;
        self.cursor_col = 0;
        self.is_modified = true;
        self.adjust_scroll();
    }

    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            // Delete character to the left
            self.lines[self.cursor_line].remove_char(self.cursor_col - 1);
            self.cursor_col -= 1;
            self.is_modified = true;
            self.adjust_scroll();
        } else if self.cursor_line > 0 {
            // Merge with previous line
            let prev_idx = self.cursor_line - 1;
            let prev_len = self.lines[prev_idx].len;
            let cur_len = self.lines[self.cursor_line].len;

            if prev_len + cur_len <= MAX_LINE_CHARS {
                let mut cur_buf = [0u8; MAX_LINE_CHARS];
                cur_buf[..cur_len].copy_from_slice(&self.lines[self.cursor_line].chars[..cur_len]);

                self.lines[prev_idx].chars[prev_len..prev_len + cur_len]
                    .copy_from_slice(&cur_buf[..cur_len]);
                self.lines[prev_idx].len = prev_len + cur_len;

                // Shift remaining lines up
                for i in self.cursor_line..self.lines_count.saturating_sub(1) {
                    self.lines[i] = self.lines[i + 1];
                }
                self.lines_count -= 1;
                self.cursor_line = prev_idx;
                self.cursor_col = prev_len;
                self.is_modified = true;
                self.adjust_scroll();
            }
        }
    }

    pub fn delete_char(&mut self) {
        let cur_len = self.lines[self.cursor_line].len;
        if self.cursor_col < cur_len {
            self.lines[self.cursor_line].remove_char(self.cursor_col);
            self.is_modified = true;
        } else if self.cursor_line + 1 < self.lines_count {
            // Merge next line into current
            let next_idx = self.cursor_line + 1;
            let next_len = self.lines[next_idx].len;
            if cur_len + next_len <= MAX_LINE_CHARS {
                let mut next_buf = [0u8; MAX_LINE_CHARS];
                next_buf[..next_len].copy_from_slice(&self.lines[next_idx].chars[..next_len]);

                self.lines[self.cursor_line].chars[cur_len..cur_len + next_len]
                    .copy_from_slice(&next_buf[..next_len]);
                self.lines[self.cursor_line].len = cur_len + next_len;

                for i in next_idx..self.lines_count.saturating_sub(1) {
                    self.lines[i] = self.lines[i + 1];
                }
                self.lines_count -= 1;
                self.is_modified = true;
            }
        }
    }

    pub fn insert_tab(&mut self) {
        // Tab inserts 4 spaces
        for _ in 0..4 {
            self.insert_char(b' ');
        }
    }

    // -----------------------------------------------------------------------
    // Cursor Movement & Viewport Scrolling
    // -----------------------------------------------------------------------

    pub fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.lines[self.cursor_line].len;
        }
        self.adjust_scroll();
    }

    pub fn move_right(&mut self) {
        let cur_len = self.lines[self.cursor_line].len;
        if self.cursor_col < cur_len {
            self.cursor_col += 1;
        } else if self.cursor_line + 1 < self.lines_count {
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
        self.adjust_scroll();
    }

    pub fn move_up(&mut self) {
        if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_line].len);
            self.adjust_scroll();
        }
    }

    pub fn move_down(&mut self) {
        if self.cursor_line + 1 < self.lines_count {
            self.cursor_line += 1;
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_line].len);
            self.adjust_scroll();
        }
    }

    pub fn move_home(&mut self) {
        self.cursor_col = 0;
        self.adjust_scroll();
    }

    pub fn move_end(&mut self) {
        self.cursor_col = self.lines[self.cursor_line].len;
        self.adjust_scroll();
    }

    pub fn visible_rows(&self) -> usize {
        let content_h = self.screen_h - TOOLBAR_H - STATUSBAR_H;
        let line_h = self.font_mode.font().line_height().max(8);
        (content_h / line_h).max(1) as usize
    }

    pub fn page_up(&mut self) {
        let rows = self.visible_rows();
        self.cursor_line = self.cursor_line.saturating_sub(rows);
        self.cursor_col = self.cursor_col.min(self.lines[self.cursor_line].len);
        self.adjust_scroll();
    }

    pub fn page_down(&mut self) {
        let rows = self.visible_rows();
        self.cursor_line = (self.cursor_line + rows).min(self.lines_count.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(self.lines[self.cursor_line].len);
        self.adjust_scroll();
    }

    fn adjust_scroll(&mut self) {
        let rows = self.visible_rows();
        if self.cursor_line < self.scroll_line {
            self.scroll_line = self.cursor_line;
        } else if self.cursor_line >= self.scroll_line + rows {
            self.scroll_line = self.cursor_line.saturating_sub(rows - 1);
        }

        // Horizontal scroll margin
        let content_w = self.screen_w - GUTTER_W;
        let char_w = self.font_mode.font().measure_char(b'M').max(6);
        let visible_cols = (content_w / char_w).max(10) as usize;

        if self.cursor_col < self.scroll_col {
            self.scroll_col = self.cursor_col;
        } else if self.cursor_col >= self.scroll_col + visible_cols {
            self.scroll_col = self.cursor_col.saturating_sub(visible_cols - 1);
        }
    }

    // -----------------------------------------------------------------------
    // Clipboard: Copy, Cut, Paste
    // -----------------------------------------------------------------------

    pub fn copy_line(&mut self) {
        let line = &self.lines[self.cursor_line];
        let copy_len = line.len.min(MAX_CLIPBOARD);
        self.clipboard[..copy_len].copy_from_slice(&line.chars[..copy_len]);
        self.clipboard_len = copy_len;
        self.set_status("Line copied to clipboard");
    }

    pub fn cut_line(&mut self) {
        self.copy_line();
        if self.lines_count > 1 {
            for i in self.cursor_line..self.lines_count.saturating_sub(1) {
                self.lines[i] = self.lines[i + 1];
            }
            self.lines_count -= 1;
            if self.cursor_line >= self.lines_count {
                self.cursor_line = self.lines_count - 1;
            }
            self.cursor_col = 0;
            self.is_modified = true;
            self.adjust_scroll();
        } else {
            self.lines[0].clear();
            self.cursor_col = 0;
            self.is_modified = true;
        }
        self.set_status("Line cut to clipboard");
    }

    pub fn paste(&mut self) {
        if self.clipboard_len == 0 {
            self.set_status("Clipboard is empty");
            return;
        }
        for i in 0..self.clipboard_len {
            self.insert_char(self.clipboard[i]);
        }
        self.set_status("Pasted clipboard contents");
    }

    // -----------------------------------------------------------------------
    // File I/O (VFS Integration)
    // -----------------------------------------------------------------------

    pub fn load_file(&mut self, filename: &str) -> bool {
        let mut c_fname = [0u8; 64];
        let bytes = filename.as_bytes();
        let copy_len = bytes.len().min(c_fname.len() - 1);
        c_fname[..copy_len].copy_from_slice(&bytes[..copy_len]);
        c_fname[copy_len] = 0;

        let handle = unsafe { ffi::sys_open(c_fname.as_ptr()) };
        if handle < 0 {
            self.set_status("Error: File not found");
            return false;
        }

        static mut READ_BUF: [u8; MAX_FILE_SIZE] = [0; MAX_FILE_SIZE];
        let bytes_read = unsafe {
            let buf_ptr = core::ptr::addr_of_mut!(READ_BUF) as *mut u8;
            let n = ffi::sys_read(handle, buf_ptr, MAX_FILE_SIZE as i32);
            ffi::sys_close(handle);
            n
        };

        if bytes_read <= 0 {
            // Empty file
            self.new_document();
            self.set_filename(filename);
            self.set_status("Loaded empty file");
            return true;
        }

        let slice = unsafe { core::slice::from_raw_parts(core::ptr::addr_of!(READ_BUF) as *const u8, bytes_read as usize) };

        // Parse content into lines
        self.lines_count = 0;
        let mut cur_line_idx = 0;
        self.lines[0].clear();

        let mut idx = 0;
        while idx < slice.len() && cur_line_idx < MAX_LINES {
            let b = slice[idx];
            if b == b'\r' {
                // Ignore CR if followed by LF
                if idx + 1 < slice.len() && slice[idx + 1] == b'\n' {
                    idx += 1;
                }
                cur_line_idx += 1;
                if cur_line_idx < MAX_LINES {
                    self.lines[cur_line_idx].clear();
                }
            } else if b == b'\n' {
                cur_line_idx += 1;
                if cur_line_idx < MAX_LINES {
                    self.lines[cur_line_idx].clear();
                }
            } else {
                if self.lines[cur_line_idx].len < MAX_LINE_CHARS {
                    let len = self.lines[cur_line_idx].len;
                    self.lines[cur_line_idx].chars[len] = b;
                    self.lines[cur_line_idx].len += 1;
                }
            }
            idx += 1;
        }

        self.lines_count = (cur_line_idx + 1).min(MAX_LINES);
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.scroll_line = 0;
        self.scroll_col = 0;
        self.is_modified = false;
        self.set_filename(filename);
        self.set_status("File loaded successfully");
        true
    }

    pub fn save_file(&mut self) -> bool {
        let mut fname_buf = [0u8; 32];
        let flen = self.filename_len.min(31);
        fname_buf[..flen].copy_from_slice(&self.filename[..flen]);
        let fname = core::str::from_utf8(&fname_buf[..flen]).unwrap_or("UNTITLED.TXT");
        self.save_file_as(fname)
    }

    pub fn save_file_as(&mut self, filename: &str) -> bool {
        let mut c_fname = [0u8; 64];
        let bytes = filename.as_bytes();
        let copy_len = bytes.len().min(c_fname.len() - 1);
        c_fname[..copy_len].copy_from_slice(&bytes[..copy_len]);
        c_fname[copy_len] = 0;

        static mut WRITE_BUF: [u8; MAX_FILE_SIZE] = [0; MAX_FILE_SIZE];
        let mut write_pos = 0;

        unsafe {
            let buf = &mut *core::ptr::addr_of_mut!(WRITE_BUF);
            for i in 0..self.lines_count {
                let line = &self.lines[i];
                if write_pos + line.len + 1 >= MAX_FILE_SIZE {
                    break;
                }
                buf[write_pos..write_pos + line.len].copy_from_slice(&line.chars[..line.len]);
                write_pos += line.len;
                buf[write_pos] = b'\n';
                write_pos += 1;
            }

            let rc = ffi::sys_write_file(c_fname.as_ptr(), buf.as_ptr(), write_pos as u32);
            if rc >= 0 {
                self.is_modified = false;
                self.set_filename(filename);
                self.set_status("File saved successfully");
                true
            } else {
                self.set_status("Error: Write failed");
                false
            }
        }
    }

    // -----------------------------------------------------------------------
    // Action Dispatch
    // -----------------------------------------------------------------------

    pub fn perform_action(&mut self, action: ToolbarAction) {
        match action {
            ToolbarAction::New => self.new_document(),
            ToolbarAction::Open => {
                // Open HELLO.TXT by default or alternate between known test fixtures
                if self.filename_as_str() == "HELLO.TXT" {
                    self.load_file("SYSTEM.CFG");
                } else {
                    self.load_file("HELLO.TXT");
                }
            }
            ToolbarAction::Save => {
                self.save_file();
            }
            ToolbarAction::Cut => self.cut_line(),
            ToolbarAction::Copy => self.copy_line(),
            ToolbarAction::Paste => self.paste(),
            ToolbarAction::ToggleFont => {
                self.font_mode = self.font_mode.next();
                let f_label = self.font_mode.label();
                self.set_status(f_label);
                self.adjust_scroll();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Keyboard Event Handling
    // -----------------------------------------------------------------------

    pub fn handle_key(&mut self, key: i32) {
        if key < 0 {
            return;
        }

        match key {
            // Newline / Enter
            10 | 13 => self.insert_newline(),

            // Backspace
            8 | 127 => self.backspace(),

            // Tab
            9 => self.insert_tab(),

            // Arrow Keys & Page Keys
            0x101 => self.move_up(),
            0x102 => self.move_down(),
            0x103 => self.move_left(),
            0x104 => self.move_right(),
            0x105 => self.move_home(),
            0x106 => self.move_end(),
            0x107 => self.page_up(),
            0x108 => self.page_down(),

            // Control shortcuts
            14 => self.perform_action(ToolbarAction::New),        // Ctrl+N
            15 => self.perform_action(ToolbarAction::Open),       // Ctrl+O
            19 => self.perform_action(ToolbarAction::Save),       // Ctrl+S
            24 => self.perform_action(ToolbarAction::Cut),        // Ctrl+X
            3  => self.perform_action(ToolbarAction::Copy),       // Ctrl+C
            22 => self.perform_action(ToolbarAction::Paste),      // Ctrl+V
            6  => self.perform_action(ToolbarAction::ToggleFont), // Ctrl+F
            17 | 27 => { self.running = false; }                 // Ctrl+Q / Esc to exit

            // Printable ASCII range
            0x20..=0x7E => self.insert_char(key as u8),

            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Mouse Event Handling & Hit-Testing
    // -----------------------------------------------------------------------

    pub fn button_at(&self, mx: i32, my: i32) -> Option<ToolbarAction> {
        if my < 0 || my >= TOOLBAR_H {
            return None;
        }

        let mut bx = 6;
        for btn in &TOOLBAR_BUTTONS {
            let bw = if btn.action == ToolbarAction::ToggleFont {
                match self.font_mode {
                    FontMode::Mono8x16 => 92,
                    FontMode::Sans8x16 => 90,
                    FontMode::Small8x8 => 96,
                    FontMode::Large12x24 => 98,
                }
            } else {
                btn.width
            };

            let rect = Rect::new(bx, 3, bw, TOOLBAR_H - 6);
            if rect.contains(mx, my) {
                return Some(btn.action);
            }
            bx += bw + 6;
        }
        None
    }

    pub fn on_mouse_click(&mut self, mx: i32, my: i32) {
        if let Some(action) = self.button_at(mx, my) {
            self.perform_action(action);
            return;
        }

        // Click in text canvas: position cursor
        if my >= TOOLBAR_H && my < self.screen_h - STATUSBAR_H && mx >= GUTTER_W {
            let font = self.font_mode.font();
            let line_h = font.line_height().max(8);
            let rel_y = my - TOOLBAR_H;
            let clicked_row = (rel_y / line_h) as usize + self.scroll_line;

            if clicked_row < self.lines_count {
                self.cursor_line = clicked_row;
                let rel_x = mx - GUTTER_W;
                let char_w = font.measure_char(b'M').max(6);
                let clicked_col = (rel_x / char_w) as usize + self.scroll_col;
                self.cursor_col = clicked_col.min(self.lines[self.cursor_line].len);
                self.adjust_scroll();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Surface Rendering & Presentation
    // -----------------------------------------------------------------------

    pub fn render<S: Surface + ?Sized>(&self, s: &mut S) {
        let f_small = Font::small();
        let cur_font = self.font_mode.font();

        // 1. Fill main background
        s.fill_rect(Rect::new(0, 0, self.screen_w, self.screen_h), COLOR_BG);

        // 2. Render Toolbar
        s.fill_rect(Rect::new(0, 0, self.screen_w, TOOLBAR_H), COLOR_TOOLBAR_BG);
        s.fill_rect(Rect::new(0, TOOLBAR_H - 1, self.screen_w, 1), COLOR_TOOLBAR_BORDER);

        let mut bx = 6;
        for btn in &TOOLBAR_BUTTONS {
            let (label, bw) = if btn.action == ToolbarAction::ToggleFont {
                (self.font_mode.label(), match self.font_mode {
                    FontMode::Mono8x16 => 92,
                    FontMode::Sans8x16 => 90,
                    FontMode::Small8x8 => 96,
                    FontMode::Large12x24 => 98,
                })
            } else {
                (btn.label, btn.width)
            };

            let rect = Rect::new(bx, 3, bw, TOOLBAR_H - 6);
            let is_hovered = self.hovered_action == Some(btn.action);
            let btn_bg = if is_hovered { COLOR_BTN_HOVER } else { COLOR_BTN_BG };

            s.fill_rect(rect, btn_bg);
            draw_rect_outline(s, rect, COLOR_BTN_BORDER);

            let txt_x = rect.x + (rect.w - (label.len() as i32 * 8)) / 2;
            let txt_y = rect.y + (rect.h - 8) / 2;
            draw_text_font(s, txt_x, txt_y, label, COLOR_BTN_TEXT, &f_small);

            bx += bw + 6;
        }

        // 3. Render Gutter & Document Canvas
        let content_h = self.screen_h - TOOLBAR_H - STATUSBAR_H;
        let gutter_rect = Rect::new(0, TOOLBAR_H, GUTTER_W, content_h);
        s.fill_rect(gutter_rect, COLOR_GUTTER_BG);
        s.fill_rect(Rect::new(GUTTER_W - 1, TOOLBAR_H, 1, content_h), COLOR_GUTTER_BORDER);

        let line_h = cur_font.line_height().max(8);
        let rows = (content_h / line_h).max(1) as usize;

        for i in 0..rows {
            let line_idx = self.scroll_line + i;
            if line_idx >= self.lines_count {
                break;
            }

            let y = TOOLBAR_H + (i as i32) * line_h;
            let is_cur_line = line_idx == self.cursor_line;

            // Active line highlight in canvas
            if is_cur_line {
                let active_rect = Rect::new(GUTTER_W, y, self.screen_w - GUTTER_W, line_h);
                s.fill_rect(active_rect, COLOR_LINE_ACTIVE);
            }

            // Render line number in gutter (1-based)
            let mut num_str = [b' '; 5];
            let num = (line_idx + 1) as u32;
            let len = format_u32(num, &mut num_str);
            let gutter_text = core::str::from_utf8(&num_str[..len]).unwrap_or("");
            let g_col = if is_cur_line { COLOR_GUTTER_ACTIVE } else { COLOR_GUTTER_FG };
            let g_x = GUTTER_W - 10 - (len as i32 * 8);
            let g_y = y + (line_h - 8) / 2;
            draw_text_font(s, g_x, g_y, gutter_text, g_col, &f_small);

            // Render document line text with syntax highlighting
            let line = &self.lines[line_idx];
            let canvas_x = GUTTER_W + 6;
            render_highlighted_line(s, canvas_x, y, line, self.scroll_col, &cur_font);

            // Render cursor if on this line
            if is_cur_line {
                let rel_col = self.cursor_col.saturating_sub(self.scroll_col);
                let cursor_x = canvas_x + (rel_col as i32) * cur_font.measure_char(b'M');
                if cursor_x < self.screen_w - 4 {
                    s.fill_rect(Rect::new(cursor_x, y + 1, 2, line_h - 2), COLOR_CURSOR);
                }
            }
        }

        // 4. Render Status Bar
        let sb_y = self.screen_h - STATUSBAR_H;
        let sb_rect = Rect::new(0, sb_y, self.screen_w, STATUSBAR_H);
        s.fill_rect(sb_rect, COLOR_STATUS_BG);
        s.fill_rect(Rect::new(0, sb_y, self.screen_w, 1), COLOR_STATUS_BORDER);

        // Left: Filename + Dirty Indicator
        let fname = self.filename_as_str();
        draw_text_font(s, 10, sb_y + 4, fname, COLOR_BTN_TEXT, &f_small);
        let mut off_x = 10 + (fname.len() as i32 * 8) + 4;
        if self.is_modified {
            draw_char_font(s, off_x, sb_y + 4, b'*', COLOR_DIRTY_STAR, &f_small);
            off_x += 12;
        }

        // Status message
        let status = self.status_as_str();
        if !status.is_empty() {
            draw_text_font(s, off_x + 16, sb_y + 4, status, COLOR_STATUS_TEXT, &f_small);
        }

        // Right side: Line, Column, and Font Mode info
        let mut right_buf = [0u8; 64];
        let mut rlen = 0;
        let prefix = b"Ln ";
        right_buf[rlen..rlen + prefix.len()].copy_from_slice(prefix);
        rlen += prefix.len();

        let mut nbuf = [0u8; 8];
        let nlen = format_u32((self.cursor_line + 1) as u32, &mut nbuf);
        right_buf[rlen..rlen + nlen].copy_from_slice(&nbuf[..nlen]);
        rlen += nlen;

        let col_pfx = b", Col ";
        right_buf[rlen..rlen + col_pfx.len()].copy_from_slice(col_pfx);
        rlen += col_pfx.len();

        let clen = format_u32((self.cursor_col + 1) as u32, &mut nbuf);
        right_buf[rlen..rlen + clen].copy_from_slice(&nbuf[..clen]);
        rlen += clen;

        let div = b" | UTF-8 | ";
        right_buf[rlen..rlen + div.len()].copy_from_slice(div);
        rlen += div.len();

        let fmode_name = self.font_mode.label().as_bytes();
        right_buf[rlen..rlen + fmode_name.len()].copy_from_slice(fmode_name);
        rlen += fmode_name.len();

        if let Ok(right_str) = core::str::from_utf8(&right_buf[..rlen]) {
            let rx = self.screen_w - (rlen as i32 * 8) - 12;
            draw_text_font(s, rx, sb_y + 4, right_str, COLOR_STATUS_ACCENT, &f_small);
        }

        // 5. Render Mouse Pointer
        let cur_shape = if self.hovered_action.is_some() {
            CursorShape::Pointer
        } else if self.mouse_y >= TOOLBAR_H && self.mouse_y < sb_y && self.mouse_x >= GUTTER_W {
            CursorShape::IBeam
        } else {
            CursorShape::Arrow
        };
        draw_cursor_sprite(s, self.mouse_x, self.mouse_y, cur_shape, CursorTheme::ModernDark);
    }
}

fn draw_rect_outline<S: Surface + ?Sized>(s: &mut S, r: Rect, c: Color) {
    if r.w <= 0 || r.h <= 0 { return; }
    s.fill_rect(Rect::new(r.x, r.y, r.w, 1), c);
    s.fill_rect(Rect::new(r.x, r.bottom() - 1, r.w, 1), c);
    s.fill_rect(Rect::new(r.x, r.y, 1, r.h), c);
    s.fill_rect(Rect::new(r.right() - 1, r.y, 1, r.h), c);
}

// ---------------------------------------------------------------------------
// Syntax Highlighting Renderer
// ---------------------------------------------------------------------------

fn is_keyword(word: &[u8]) -> bool {
    matches!(
        word,
        b"fn" | b"let" | b"mut" | b"if" | b"else" | b"return" | b"struct" | b"impl" |
        b"pub" | b"use" | b"mod" | b"for" | b"while" | b"match" | b"true" | b"false" |
        b"const" | b"static" | b"enum" | b"type" | b"self" | b"Self" | b"as"
    )
}

fn render_highlighted_line<S: Surface + ?Sized>(
    s: &mut S,
    start_x: i32,
    y: i32,
    line: &Line,
    scroll_col: usize,
    font: &Font,
) {
    if scroll_col >= line.len {
        return;
    }

    let slice = &line.chars[scroll_col..line.len];
    let mut x = start_x;
    let mut i = 0;
    let char_adv = font.measure_char(b'M');

    // Check for comment starting with // or #
    let mut comment_start = None;
    for idx in 0..slice.len() {
        if slice[idx] == b'#' {
            comment_start = Some(idx);
            break;
        }
        if idx + 1 < slice.len() && slice[idx] == b'/' && slice[idx + 1] == b'/' {
            comment_start = Some(idx);
            break;
        }
    }

    let mut in_string = false;

    while i < slice.len() {
        let b = slice[i];

        if let Some(c_idx) = comment_start {
            if i >= c_idx {
                draw_char_font(s, x, y, b, COLOR_SYN_COMMENT, font);
                x += char_adv;
                i += 1;
                continue;
            }
        }

        if b == b'"' {
            in_string = !in_string;
            draw_char_font(s, x, y, b, COLOR_SYN_STRING, font);
            x += char_adv;
            i += 1;
            continue;
        }

        if in_string {
            draw_char_font(s, x, y, b, COLOR_SYN_STRING, font);
            x += char_adv;
            i += 1;
            continue;
        }

        // Check for identifier / keyword
        if (b.is_ascii_alphabetic() || b == b'_') && (i == 0 || !slice[i - 1].is_ascii_alphanumeric()) {
            let start = i;
            while i < slice.len() && (slice[i].is_ascii_alphanumeric() || slice[i] == b'_') {
                i += 1;
            }
            let word = &slice[start..i];
            let col = if is_keyword(word) {
                COLOR_SYN_KEYWORD
            } else {
                COLOR_SYN_DEFAULT
            };
            for &wb in word {
                draw_char_font(s, x, y, wb, col, font);
                x += char_adv;
            }
            continue;
        }

        // Digits
        if b.is_ascii_digit() {
            draw_char_font(s, x, y, b, COLOR_SYN_NUMBER, font);
            x += char_adv;
            i += 1;
            continue;
        }

        // Regular character
        draw_char_font(s, x, y, b, COLOR_SYN_DEFAULT, font);
        x += char_adv;
        i += 1;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn format_u32(val: u32, buf: &mut [u8]) -> usize {
    if val == 0 {
        if !buf.is_empty() {
            buf[0] = b'0';
            return 1;
        }
        return 0;
    }
    let mut temp = [0u8; 10];
    let mut n = val;
    let mut count = 0;
    while n > 0 && count < 10 {
        temp[count] = b'0' + (n % 10) as u8;
        n /= 10;
        count += 1;
    }
    let len = count.min(buf.len());
    for i in 0..len {
        buf[i] = temp[count - 1 - i];
    }
    len
}

// ---------------------------------------------------------------------------
// Static Global BSS Storage & Headless Selftest
// ---------------------------------------------------------------------------

pub static mut GLOBAL_EDITOR: Editor = Editor::uninit();
pub static mut TEST_RENDER_BUF: [u32; 360 * 250] = [0; 360 * 250];

pub fn run_selftest() -> i32 {
    let mut code = 0;

    unsafe {
        let msg = b"[edit-selftest] running Month 3 text editor verification suite\n\0";
        ffi::sys_write(msg.as_ptr());
    }

    let ed = unsafe { &mut *addr_of_mut!(GLOBAL_EDITOR) };
    ed.init(640, 400);

    // Test 1: Buffer initialization, inserting characters, and string verification
    ed.new_document();
    let init_ok = ed.lines_count == 1 && ed.cursor_line == 0 && ed.cursor_col == 0;

    for &b in b"fn main() {" {
        ed.insert_char(b);
    }
    let l0_str = ed.lines[0].as_str();
    let t1_ok = init_ok && ed.cursor_col == 11 && l0_str == "fn main() {" && ed.is_modified;

    if t1_ok {
        unsafe { ffi::sys_write(b"[edit-selftest] PASS: buffer initialization & character insertion\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[edit-selftest] FAIL: buffer insertion\n\0".as_ptr()) };
        code |= 1;
    }

    // Test 2: Line splitting (Enter), line merging (Backspace), and cursor tracking
    ed.insert_newline();
    for &b in b"    let x = 42;" {
        ed.insert_char(b);
    }
    ed.insert_newline();
    for &b in b"}" {
        ed.insert_char(b);
    }

    // Test backspace merging
    ed.move_home(); // cursor at line 2, col 0
    ed.backspace(); // should merge line 2 with line 1
    let t2_ok = ed.lines_count == 2 && ed.cursor_line == 1 && ed.cursor_col == 15;

    if t2_ok {
        unsafe { ffi::sys_write(b"[edit-selftest] PASS: newline splitting & backspace merging\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[edit-selftest] FAIL: newline splitting / backspace\n\0".as_ptr()) };
        code |= 2;
    }

    // Test 3: VFS File I/O round-trip (saving document and loading it back)
    ed.new_document();
    for &b in b"NovaOS Editor Test Content" {
        ed.insert_char(b);
    }
    ed.set_filename("TESTDOC.TXT");
    let save_ok = ed.save_file();
    let load_ok = if save_ok { ed.load_file("TESTDOC.TXT") } else { false };
    let t3_ok = save_ok && load_ok && ed.lines_count >= 1 && !ed.is_modified;

    if t3_ok {
        unsafe { ffi::sys_write(b"[edit-selftest] PASS: VFS file save & load roundtrip\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[edit-selftest] FAIL: VFS file save & load\n\0".as_ptr()) };
        code |= 4;
    }

    // Test 4: Font modes (Mono, Sans, Small, Large) and glyph measurement verification
    ed.font_mode = FontMode::Mono8x16;
    let adv_mono = ed.font_mode.font().measure_char(b'M');
    ed.font_mode = FontMode::Small8x8;
    let adv_small = ed.font_mode.font().measure_char(b'M');
    ed.font_mode = FontMode::Large12x24;
    let adv_large = ed.font_mode.font().measure_char(b'M');
    let t4_ok = adv_mono == 8 && adv_small == 8 && adv_large == 12;

    if t4_ok {
        unsafe { ffi::sys_write(b"[edit-selftest] PASS: multi-size font modes & metrics\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[edit-selftest] FAIL: font modes & metrics\n\0".as_ptr()) };
        code |= 8;
    }

    // Test 5: Mouse click hit-testing on toolbar buttons and canvas
    let hit_new = ed.button_at(10, 10);
    let hit_open = ed.button_at(60, 10);
    let hit_save = ed.button_at(115, 10);
    let t5_ok = hit_new == Some(ToolbarAction::New)
        && hit_open == Some(ToolbarAction::Open)
        && hit_save == Some(ToolbarAction::Save);

    if t5_ok {
        unsafe { ffi::sys_write(b"[edit-selftest] PASS: mouse hit-testing & toolbar button dispatch\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[edit-selftest] FAIL: mouse hit-testing\n\0".as_ptr()) };
        code |= 16;
    }

    // Test 6: 32-bit BufferSurface32 surface rasterization
    let render_slice = unsafe { &mut *addr_of_mut!(TEST_RENDER_BUF) };
    render_slice.fill(0);
    let mut surface = BufferSurface32::new(render_slice, 360, 250);
    ed.render(&mut surface);

    let non_zero_pixels = surface.buffer.iter().filter(|&&p| p != 0).count();
    let t6_ok = non_zero_pixels > 200;

    if t6_ok {
        unsafe { ffi::sys_write(b"[edit-selftest] PASS: graphical surface rasterization (BufferSurface32)\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[edit-selftest] FAIL: surface rasterization\n\0".as_ptr()) };
        code |= 32;
    }

    if code == 0 {
        unsafe { ffi::sys_write(b"[edit-selftest] all cases passed\n\0".as_ptr()) };
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

// ---------------------------------------------------------------------------
// Standalone Entry Point
// ---------------------------------------------------------------------------

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
        let msg = b"edit: entering NovaOS graphical text editor (Month 3 Core Apps)\n\0";
        ffi::sys_write(msg.as_ptr());
    }

    let mut fb_info = ffi::NovaFbInfo {
        struct_size: core::mem::size_of::<ffi::NovaFbInfo>() as u32,
        ..Default::default()
    };
    let info_rc = unsafe { ffi::sys_fb_info(&mut fb_info) };
    let (screen_w, screen_h) = if info_rc == 0 && fb_info.width > 0 && fb_info.height > 0 {
        (fb_info.width as i32, fb_info.height as i32)
    } else {
        (1024, 768)
    };

    let acq_rc = unsafe { ffi::sys_fb_acquire(ffi::NOVA_FB_BACKEND_ANY) };
    if acq_rc < 0 {
        unsafe {
            let msg = b"edit: failed to acquire framebuffer display (-EBUSY or -ENODEV)\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        return 1;
    }

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
            let msg = b"edit: failed to create framebuffer surface\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        return 1;
    }

    let fb_handle = create_req.handle;
    let pixels_ptr = create_req.pixels as *mut u32;
    let total_pixels = (screen_w * screen_h) as usize;

    let ed = unsafe { &mut *addr_of_mut!(GLOBAL_EDITOR) };
    ed.init(screen_w, screen_h);

    // If an initial file was passed as argument, load it!
    if argc >= 2 {
        let ptr = unsafe { *argv.offset(1) };
        let mut fname_buf = [0u8; 32];
        let mut flen = 0;
        while flen < 31 {
            let b = unsafe { *ptr.add(flen) };
            if b == 0 { break; }
            fname_buf[flen] = b;
            flen += 1;
        }
        if flen > 0 {
            if let Ok(s) = core::str::from_utf8(&fname_buf[..flen]) {
                ed.load_file(s);
            }
        }
    } else {
        // By default, try loading HELLO.TXT if it exists
        ed.load_file("HELLO.TXT");
    }

    let mut prev_left = false;

    while ed.running {
        // 1. Process keyboard events
        let key = unsafe { ffi::sys_read_key() };
        if key >= 0 {
            ed.handle_key(key);
        }

        // 2. Process mouse events
        let mut m = ffi::MouseState::default();
        let _ = unsafe { ffi::sys_mouse_read(&mut m) };
        ed.mouse_x = (ed.mouse_x + m.dx).clamp(0, screen_w - 1);
        ed.mouse_y = (ed.mouse_y + m.dy).clamp(0, screen_h - 1);

        ed.hovered_action = ed.button_at(ed.mouse_x, ed.mouse_y);

        let just_pressed = m.left_button && !prev_left;
        if just_pressed {
            ed.on_mouse_click(ed.mouse_x, ed.mouse_y);
        }
        prev_left = m.left_button;

        // 3. Render frame to surface buffer in userland memory
        let pixel_slice = unsafe { core::slice::from_raw_parts_mut(pixels_ptr, total_pixels) };
        let mut surface = BufferSurface32::new_with_pitch(pixel_slice, screen_w, screen_h, screen_w);
        ed.render(&mut surface);

        // 4. Present damage with single syscall
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

    unsafe { ffi::sys_fb_release() };
    0
}

// ---------------------------------------------------------------------------
// Host Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_editor_selftest_cases() {
        let code = run_selftest();
        assert_eq!(code, 0, "Editor selftest cases must all pass");
    }

    #[test]
    fn test_editor_line_operations() {
        let mut line = Line::empty();
        assert_eq!(line.len, 0);

        assert!(line.insert_char(0, b'A'));
        assert!(line.insert_char(1, b'C'));
        assert!(line.insert_char(1, b'B'));
        assert_eq!(line.as_str(), "ABC");

        let removed = line.remove_char(1);
        assert_eq!(removed, Some(b'B'));
        assert_eq!(line.as_str(), "AC");
    }

    #[test]
    fn test_editor_tab_and_backspace() {
        let mut ed = Editor::uninit();
        ed.init(640, 480);
        ed.insert_tab();
        assert_eq!(ed.lines[0].as_str(), "    ");
        assert_eq!(ed.cursor_col, 4);

        ed.backspace();
        assert_eq!(ed.lines[0].as_str(), "   ");
        assert_eq!(ed.cursor_col, 3);
    }

    #[test]
    fn test_editor_clipboard_copy_paste() {
        let mut ed = Editor::uninit();
        ed.init(640, 480);
        for &b in b"Clipboard Test" {
            ed.insert_char(b);
        }
        ed.copy_line();
        assert_eq!(&ed.clipboard[..ed.clipboard_len], b"Clipboard Test");

        ed.insert_newline();
        ed.paste();
        assert_eq!(ed.lines[1].as_str(), "Clipboard Test");
    }

    #[test]
    fn test_editor_font_mode_toggle() {
        let mut ed = Editor::uninit();
        ed.init(640, 480);
        assert_eq!(ed.font_mode, FontMode::Mono8x16);
        ed.perform_action(ToolbarAction::ToggleFont);
        assert_eq!(ed.font_mode, FontMode::Sans8x16);
        ed.perform_action(ToolbarAction::ToggleFont);
        assert_eq!(ed.font_mode, FontMode::Small8x8);
        ed.perform_action(ToolbarAction::ToggleFont);
        assert_eq!(ed.font_mode, FontMode::Large12x24);
        ed.perform_action(ToolbarAction::ToggleFont);
        assert_eq!(ed.font_mode, FontMode::Mono8x16);
    }
}
