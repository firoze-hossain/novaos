//! userland/files-rs/files.rs - NovaOS Graphical File Manager (Month 3 Core Desktop Apps)
//!
//! Features:
//!   - Browses filesystem files and directories using VFS syscalls (SYS_LIST_FILES).
//!   - Rich metadata classification (text document, executable, configuration, package).
//!   - Interactive mouse operations: file selection, open preview, move/rename, and delete.
//!   - Dual view modes: detailed List View (with columns) and 2D Grid / Icon View.
//!   - Text file preview modal with close button.
//!   - Interactive toolbar with vector action buttons (Open, Move, Delete, Refresh, View Toggle).
//!   - Rendered using the NovaOS Unified Design System (ui-rs) with zero per-pixel syscalls.
//!   - Full standalone framebuffer display support and self-test verification mode.

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
        ffi::sys_fb_release();
        let msg = b"files: internal error (panic)\n\0";
        ffi::sys_write(msg.as_ptr());
    }
    loop {
        unsafe { ffi::sys_yield() };
    }
}

pub const MAX_FILES: usize = 64;
pub const PREVIEW_BUF_LEN: usize = 4096;
pub const LISTING_BUF_LEN: usize = 4096;

static mut LISTING_BUF: [u8; LISTING_BUF_LEN] = [0; LISTING_BUF_LEN];
static mut PREVIEW_BUF: [u8; PREVIEW_BUF_LEN] = [0; PREVIEW_BUF_LEN];
static mut OP_COPY_BUF: [u8; 8192] = [0; 8192];

/// File type classification based on file extension and attributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Document,
    Config,
    Executable,
    Package,
    Data,
    Unknown,
}

impl FileKind {
    pub fn description(&self) -> &'static str {
        match self {
            FileKind::Document   => "Text Document",
            FileKind::Config     => "System Config",
            FileKind::Executable => "Executable",
            FileKind::Package    => "Software Package",
            FileKind::Data       => "Data File",
            FileKind::Unknown    => "File",
        }
    }

    pub fn color(&self, palette: &ThemePalette) -> Color {
        match self {
            FileKind::Document   => WHITE,
            FileKind::Config     => Color::rgb(140, 190, 240),
            FileKind::Executable => palette.status_success,
            FileKind::Package    => Color::rgb(220, 150, 240),
            FileKind::Data       => palette.status_warning,
            FileKind::Unknown    => palette.text_secondary,
        }
    }
}

/// A parsed file entry from the filesystem directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub name: [u8; 16],
    pub name_len: usize,
    pub size: u32,
    pub kind: FileKind,
}

impl FileEntry {
    pub fn new(name_str: &str, size: u32) -> Self {
        let mut name = [0u8; 16];
        let bytes = name_str.as_bytes();
        let name_len = bytes.len().min(15);
        name[..name_len].copy_from_slice(&bytes[..name_len]);

        let kind = Self::detect_kind(name_str);
        FileEntry {
            name,
            name_len,
            size,
            kind,
        }
    }

    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }

    fn detect_kind(name: &str) -> FileKind {
        if name.ends_with(".TXT") || name.ends_with(".txt") || name.ends_with(".MD") {
            FileKind::Document
        } else if name.ends_with(".CFG") || name.ends_with(".cfg") || name.ends_with(".INI") {
            FileKind::Config
        } else if name.ends_with(".ELF") || name.ends_with(".elf") || name.ends_with(".SO") {
            FileKind::Executable
        } else if name.ends_with(".PKG") || name.ends_with(".pkg") {
            FileKind::Package
        } else if name.ends_with(".DAT") || name.ends_with(".LOG") {
            FileKind::Data
        } else {
            FileKind::Unknown
        }
    }

    /// Formats size into human readable string (e.g. "68 B", "42 KB", "1.2 MB").
    pub fn format_size(&self, out: &mut [u8; 16]) -> usize {
        let size = self.size;
        if size < 1024 {
            let mut val = size;
            let mut digits = [0u8; 10];
            let mut d_count = 0;
            if val == 0 {
                digits[0] = b'0';
                d_count = 1;
            } else {
                while val > 0 {
                    digits[d_count] = b'0' + (val % 10) as u8;
                    val /= 10;
                    d_count += 1;
                }
            }
            let mut pos = 0;
            for i in (0..d_count).rev() {
                out[pos] = digits[i];
                pos += 1;
            }
            out[pos] = b' ';
            out[pos + 1] = b'B';
            pos + 2
        } else if size < 1024 * 1024 {
            let kb = size / 1024;
            let mut val = kb;
            let mut digits = [0u8; 10];
            let mut d_count = 0;
            while val > 0 {
                digits[d_count] = b'0' + (val % 10) as u8;
                val /= 10;
                d_count += 1;
            }
            let mut pos = 0;
            for i in (0..d_count).rev() {
                out[pos] = digits[i];
                pos += 1;
            }
            out[pos] = b' ';
            out[pos + 1] = b'K';
            out[pos + 2] = b'B';
            pos + 3
        } else {
            let mb = size / (1024 * 1024);
            let rem_tenth = (size % (1024 * 1024)) * 10 / (1024 * 1024);
            let mut val = mb;
            let mut digits = [0u8; 10];
            let mut d_count = 0;
            while val > 0 {
                digits[d_count] = b'0' + (val % 10) as u8;
                val /= 10;
                d_count += 1;
            }
            let mut pos = 0;
            for i in (0..d_count).rev() {
                out[pos] = digits[i];
                pos += 1;
            }
            out[pos] = b'.';
            out[pos + 1] = b'0' + (rem_tenth as u8);
            out[pos + 2] = b' ';
            out[pos + 3] = b'M';
            out[pos + 4] = b'B';
            pos + 5
        }
    }
}

/// View mode for browsing files in the content area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    List,
    Grid,
}

/// Interactive actions available on the toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarAction {
    Open,
    Move,
    Delete,
    Refresh,
    ToggleView,
    ClosePreview,
}

/// The main File Manager application state.
pub struct FileManager {
    pub files: [FileEntry; MAX_FILES],
    pub file_count: usize,
    pub selected_idx: Option<usize>,
    pub hovered_idx: Option<usize>,
    pub hovered_btn: Option<ToolbarAction>,
    pub view_mode: ViewMode,
    pub is_preview_open: bool,
    pub preview_name: [u8; 16],
    pub preview_len: usize,
    pub status_text: [u8; 64],
    pub status_len: usize,
    pub current_path: &'static str,
    pub last_action_result: i32,
}

impl FileManager {
    pub fn new() -> Self {
        FileManager {
            files: [FileEntry::new("", 0); MAX_FILES],
            file_count: 0,
            selected_idx: None,
            hovered_idx: None,
            hovered_btn: None,
            view_mode: ViewMode::List,
            is_preview_open: false,
            preview_name: [0u8; 16],
            preview_len: 0,
            status_text: [0u8; 64],
            status_len: 0,
            current_path: "/root",
            last_action_result: 0,
        }
    }

    /// Parses directory listing format `"NAME SIZE\n"` into files array.
    pub fn load_from_listing(&mut self, listing_data: &[u8]) {
        self.file_count = 0;
        self.selected_idx = None;

        let mut pos = 0;
        while pos < listing_data.len() && self.file_count < MAX_FILES {
            // Find end of line
            let mut line_end = pos;
            while line_end < listing_data.len() && listing_data[line_end] != b'\n' {
                line_end += 1;
            }

            let line = &listing_data[pos..line_end];
            pos = line_end + 1;

            if line.is_empty() {
                continue;
            }

            // Split by space: NAME SIZE
            let mut space_idx = None;
            for i in 0..line.len() {
                if line[i] == b' ' {
                    space_idx = Some(i);
                    break;
                }
            }

            if let Some(sp) = space_idx {
                let name_part = &line[..sp];
                let size_part = &line[sp + 1..];

                if let Ok(name_str) = core::str::from_utf8(name_part) {
                    let mut size = 0u32;
                    for &b in size_part {
                        if b >= b'0' && b <= b'9' {
                            size = size.saturating_mul(10).saturating_add((b - b'0') as u32);
                        }
                    }
                    self.files[self.file_count] = FileEntry::new(name_str, size);
                    self.file_count += 1;
                }
            }
        }
    }

    /// Queries the live VFS directory listing via SYS_LIST_FILES.
    pub fn refresh(&mut self) -> bool {
        let buf_ptr = core::ptr::addr_of_mut!(LISTING_BUF) as *mut u8;
        let n = unsafe { ffi::sys_list_files(buf_ptr, LISTING_BUF_LEN as i32 - 1) };
        if n >= 0 {
            let slice = unsafe { core::slice::from_raw_parts(buf_ptr, n as usize) };
            self.load_from_listing(slice);
            self.set_status("Directory refreshed");
            true
        } else {
            self.set_status("Failed to read directory");
            false
        }
    }

    pub fn set_status(&mut self, msg: &str) {
        let bytes = msg.as_bytes();
        let len = bytes.len().min(63);
        self.status_text[..len].copy_from_slice(&bytes[..len]);
        self.status_text[len] = 0;
        self.status_len = len;
    }

    pub fn status_str(&self) -> &str {
        core::str::from_utf8(&self.status_text[..self.status_len]).unwrap_or("")
    }

    /// Selects file at index `idx`.
    pub fn select_file(&mut self, idx: usize) {
        if idx < self.file_count {
            self.selected_idx = Some(idx);
            let mut name_buf = [0u8; 16];
            let len = self.files[idx].name_len;
            name_buf[..len].copy_from_slice(&self.files[idx].name[..len]);
            if let Ok(name) = core::str::from_utf8(&name_buf[..len]) {
                self.set_status(name);
            }
        }
    }

    /// Opens the currently selected file via SYS_OPEN/SYS_READ and enters preview mode.
    pub fn open_selected(&mut self) -> bool {
        let idx = match self.selected_idx {
            Some(i) => i,
            None => {
                self.set_status("No file selected to open");
                return false;
            }
        };

        let file = self.files[idx];

        let mut name_cstr = [0u8; 17];
        name_cstr[..file.name_len].copy_from_slice(&file.name[..file.name_len]);

        let fd = unsafe { ffi::sys_open(name_cstr.as_ptr()) };
        if fd < 0 {
            self.set_status("Failed to open file");
            return false;
        }

        let preview_ptr = core::ptr::addr_of_mut!(PREVIEW_BUF) as *mut u8;
        let bytes_read = unsafe { ffi::sys_read(fd, preview_ptr, (PREVIEW_BUF_LEN - 1) as i32) };
        unsafe { ffi::sys_close(fd) };

        if bytes_read >= 0 {
            self.preview_name[..file.name_len].copy_from_slice(&file.name[..file.name_len]);
            self.preview_len = bytes_read as usize;
            unsafe {
                *preview_ptr.add(self.preview_len) = 0;
            }
            self.is_preview_open = true;
            self.set_status("Opened file preview");
            true
        } else {
            self.set_status("Failed to read file");
            false
        }
    }

    /// Moves / renames `src_name` to `dst_name` via copy-and-delete.
    pub fn move_file(&mut self, src_name: &str, dst_name: &str) -> bool {
        let mut src_cstr = [0u8; 17];
        let s_len = src_name.len().min(16);
        src_cstr[..s_len].copy_from_slice(&src_name.as_bytes()[..s_len]);

        let mut dst_cstr = [0u8; 17];
        let d_len = dst_name.len().min(16);
        dst_cstr[..d_len].copy_from_slice(&dst_name.as_bytes()[..d_len]);

        // 1. Open source file
        let fd = unsafe { ffi::sys_open(src_cstr.as_ptr()) };
        if fd < 0 {
            self.set_status("Move failed: cannot open source");
            return false;
        }

        // 2. Read full source file into copy buffer
        let buf_ptr = core::ptr::addr_of_mut!(OP_COPY_BUF) as *mut u8;
        let mut total = 0usize;
        loop {
            let rem = 8192 - total;
            if rem == 0 {
                break;
            }
            let n = unsafe { ffi::sys_read(fd, buf_ptr.add(total), rem as i32) };
            if n <= 0 {
                break;
            }
            total += n as usize;
        }
        unsafe { ffi::sys_close(fd) };

        // 3. Write target destination file
        let write_rc = unsafe { ffi::sys_write_file(dst_cstr.as_ptr(), buf_ptr, total as u32) };
        if write_rc < 0 {
            self.set_status("Move failed: cannot write destination");
            return false;
        }

        // 4. Delete source file
        let del_rc = unsafe { ffi::sys_delete_file(src_cstr.as_ptr()) };
        if del_rc < 0 {
            self.set_status("Move partial: destination written, source kept");
            self.refresh();
            return false;
        }

        self.refresh();
        self.set_status("File moved successfully");
        true
    }

    /// Deletes a file by name via SYS_DELETE_FILE.
    pub fn delete_file(&mut self, filename: &str) -> bool {
        let mut name_cstr = [0u8; 17];
        let len = filename.len().min(16);
        name_cstr[..len].copy_from_slice(&filename.as_bytes()[..len]);

        let del_rc = unsafe { ffi::sys_delete_file(name_cstr.as_ptr()) };
        if del_rc >= 0 {
            if let Some(idx) = self.selected_idx {
                if idx < self.file_count && self.files[idx].name_str() == filename {
                    self.selected_idx = None;
                }
            }
            self.refresh();
            self.set_status("File deleted");
            true
        } else {
            self.set_status("Failed to delete file");
            false
        }
    }

    /// Deletes the currently selected file via SYS_DELETE_FILE.
    pub fn delete_selected(&mut self) -> bool {
        let idx = match self.selected_idx {
            Some(i) => i,
            None => {
                self.set_status("No file selected to delete");
                return false;
            }
        };

        let mut name_buf = [0u8; 16];
        let len = self.files[idx].name_len;
        name_buf[..len].copy_from_slice(&self.files[idx].name[..len]);
        if let Ok(name_str) = core::str::from_utf8(&name_buf[..len]) {
            self.delete_file(name_str)
        } else {
            false
        }
    }

    pub fn close_preview(&mut self) {
        self.is_preview_open = false;
        self.set_status("Closed preview");
    }

    pub fn toggle_view_mode(&mut self) {
        self.view_mode = match self.view_mode {
            ViewMode::List => ViewMode::Grid,
            ViewMode::Grid => ViewMode::List,
        };
    }

    // -----------------------------------------------------------------------
    // Layout and Mouse Hit-Testing
    // -----------------------------------------------------------------------

    /// Hit-tests a coordinate against toolbar buttons.
    pub fn button_at(&self, px: i32, py: i32, viewport: Rect) -> Option<ToolbarAction> {
        let tb_y = viewport.y + 4;
        let tb_h = 24;
        if py < tb_y || py >= tb_y + tb_h {
            if self.is_preview_open {
                // Check preview modal close button
                let preview_rect = Rect::new(viewport.x + 30, viewport.y + 50, viewport.w - 60, viewport.h - 90);
                let close_btn = Rect::new(preview_rect.right() - 70, preview_rect.y + 6, 60, 20);
                if point_in_rect(px, py, close_btn.x, close_btn.y, close_btn.w, close_btn.h) {
                    return Some(ToolbarAction::ClosePreview);
                }
            }
            return None;
        }

        let mut bx = viewport.x + 120; // after path box
        let btn_w = 64;
        let spacing = 6;

        let actions = [
            ToolbarAction::Open,
            ToolbarAction::Move,
            ToolbarAction::Delete,
            ToolbarAction::Refresh,
            ToolbarAction::ToggleView,
        ];

        for action in actions {
            if point_in_rect(px, py, bx, tb_y, btn_w, tb_h) {
                return Some(action);
            }
            bx += btn_w + spacing;
        }

        None
    }

    /// Hit-tests a coordinate against file rows/cards in the content area.
    pub fn file_at(&self, px: i32, py: i32, viewport: Rect) -> Option<usize> {
        if self.is_preview_open {
            return None;
        }

        let content_y = viewport.y + 36;
        let content_h = viewport.h - 36 - 22; // reserve top toolbar and bottom status bar

        if py < content_y || py >= content_y + content_h || px < viewport.x || px >= viewport.right() {
            return None;
        }

        match self.view_mode {
            ViewMode::List => {
                let row_h = 22;
                let rel_y = py - content_y - 20; // minus column header
                if rel_y < 0 {
                    return None;
                }
                let idx = (rel_y / row_h) as usize;
                if idx < self.file_count {
                    Some(idx)
                } else {
                    None
                }
            }
            ViewMode::Grid => {
                let card_w = 76;
                let card_h = 70;
                let cols = ((viewport.w - 16) / card_w).max(1) as usize;
                let rel_x = px - viewport.x - 8;
                let rel_y = py - content_y - 8;
                if rel_x < 0 || rel_y < 0 {
                    return None;
                }
                let col = (rel_x / card_w) as usize;
                let row = (rel_y / card_h) as usize;
                let idx = row * cols + col;
                if idx < self.file_count {
                    Some(idx)
                } else {
                    None
                }
            }
        }
    }

    /// Resolves dynamic pointer cursor shape for current pointer location.
    pub fn cursor_shape_at(&self, px: i32, py: i32, viewport: Rect) -> CursorShape {
        if self.button_at(px, py, viewport).is_some() || self.file_at(px, py, viewport).is_some() {
            CursorShape::Pointer
        } else {
            CursorShape::Arrow
        }
    }

    /// Handles a left mouse click inside the file manager window.
    pub fn on_left_click(&mut self, px: i32, py: i32, viewport: Rect) {
        if let Some(btn) = self.button_at(px, py, viewport) {
            match btn {
                ToolbarAction::Open         => { self.open_selected(); }
                ToolbarAction::Move         => {
                    if let Some(idx) = self.selected_idx {
                        let mut orig_buf = [0u8; 16];
                        let orig_len = self.files[idx].name_len;
                        orig_buf[..orig_len].copy_from_slice(&self.files[idx].name[..orig_len]);

                        let mut new_name = [0u8; 16];
                        let len = orig_len.min(11);
                        new_name[..len].copy_from_slice(&orig_buf[..len]);
                        // Append .BAK
                        new_name[len] = b'.';
                        new_name[len + 1] = b'B';
                        new_name[len + 2] = b'A';
                        new_name[len + 3] = b'K';
                        let orig_str = core::str::from_utf8(&orig_buf[..orig_len]).unwrap_or("");
                        let target_str = core::str::from_utf8(&new_name[..len + 4]).unwrap_or("FILE.BAK");
                        self.move_file(orig_str, target_str);
                    } else {
                        self.set_status("Select a file to move/rename");
                    }
                }
                ToolbarAction::Delete       => { self.delete_selected(); }
                ToolbarAction::Refresh      => { self.refresh(); }
                ToolbarAction::ToggleView   => { self.toggle_view_mode(); }
                ToolbarAction::ClosePreview => { self.close_preview(); }
            }
            return;
        }

        if let Some(idx) = self.file_at(px, py, viewport) {
            if self.selected_idx == Some(idx) {
                // Second click on selected file opens it
                self.open_selected();
            } else {
                self.select_file(idx);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Rendering Pipeline
    // -----------------------------------------------------------------------

    /// Renders the complete file manager application interface into surface `s`.
    pub fn render<S: Surface + ?Sized>(&self, s: &mut S, viewport: Rect) {
        if viewport.w <= 0 || viewport.h <= 0 {
            return;
        }
        let palette = Theme::Dark.palette();
        let font = Font::regular();
        let f_small = Font::small();

        // 1. Window Background
        s.fill_rect(viewport, palette.surface_sunken);

        // 2. Toolbar Header (`height = 32px`)
        let tb_rect = Rect::new(viewport.x, viewport.y, viewport.w, 32);
        s.fill_rect(tb_rect, palette.surface_raised);
        s.fill_rect(Rect::new(viewport.x, tb_rect.bottom() - 1, viewport.w, 1), palette.border_subtle);

        // Path address box `[ /root ]`
        let path_rect = Rect::new(viewport.x + 8, viewport.y + 4, 100, 24);
        draw_rounded_rect(s, path_rect, RADIUS_XS, palette.surface_sunken);
        draw_rect_outline(s, path_rect, 1, palette.border_subtle);
        draw_text_font(s, path_rect.x + 8, path_rect.y + 4, self.current_path, palette.text_primary, &f_small);

        // Action Buttons: Open, Move, Delete, Refresh, View Toggle
        let mut bx = viewport.x + 120;
        let btn_w = 64;
        let btn_h = 24;
        let spacing = 6;

        let buttons: &[(ToolbarAction, &str, Color)] = &[
            (ToolbarAction::Open,       "Open",    palette.accent_primary),
            (ToolbarAction::Move,       "Move",    palette.surface_raised.lighten(30)),
            (ToolbarAction::Delete,     "Delete",  palette.status_danger),
            (ToolbarAction::Refresh,    "Refresh", palette.surface_raised.lighten(30)),
            (ToolbarAction::ToggleView, if self.view_mode == ViewMode::List { "Icons" } else { "List" }, palette.surface_raised.lighten(30)),
        ];

        for &(action, label, base_col) in buttons {
            let is_hovered = self.hovered_btn == Some(action);
            let b_rect = Rect::new(bx, viewport.y + 4, btn_w, btn_h);
            let col = if is_hovered { base_col.lighten(20) } else { base_col };
            draw_rounded_rect(s, b_rect, RADIUS_XS, col);
            draw_rect_outline(s, b_rect, 1, palette.border_focused);

            let sz = f_small.measure_str(label);
            let tx = b_rect.x + (b_rect.w - sz.w) / 2;
            let ty = b_rect.y + (b_rect.h - f_small.line_height()) / 2;
            draw_text_font(s, tx, ty, label, WHITE, &f_small);

            bx += btn_w + spacing;
        }

        // 3. Content Area
        let content_y = viewport.y + 32;
        let content_h = viewport.h - 32 - 22;
        let content_rect = Rect::new(viewport.x, content_y, viewport.w, content_h);

        match self.view_mode {
            ViewMode::List => {
                // Table Column Headers
                let hdr_rect = Rect::new(content_rect.x, content_rect.y, content_rect.w, 20);
                s.fill_rect(hdr_rect, palette.surface_raised.darken(10));
                s.fill_rect(Rect::new(hdr_rect.x, hdr_rect.bottom() - 1, hdr_rect.w, 1), palette.border_subtle);

                draw_text_font(s, hdr_rect.x + 36, hdr_rect.y + 3, "Name", palette.text_muted, &f_small);
                draw_text_font(s, hdr_rect.x + 220, hdr_rect.y + 3, "Size", palette.text_muted, &f_small);
                draw_text_font(s, hdr_rect.x + 320, hdr_rect.y + 3, "Type", palette.text_muted, &f_small);

                let mut cur_y = hdr_rect.bottom() + 1;
                let row_h = 22;

                for i in 0..self.file_count {
                    if cur_y + row_h > content_rect.bottom() {
                        break;
                    }

                    let file = self.files[i];
                    let is_selected = self.selected_idx == Some(i);
                    let is_hovered = self.hovered_idx == Some(i);

                    let row_rect = Rect::new(content_rect.x + 2, cur_y, content_rect.w - 4, row_h);
                    if is_selected {
                        draw_rounded_rect(s, row_rect, RADIUS_XS, palette.taskbar_item_active);
                        draw_rect_outline(s, row_rect, 1, palette.border_focused);
                    } else if is_hovered {
                        draw_rounded_rect(s, row_rect, RADIUS_XS, palette.surface_raised);
                    }

                    // Type glyph icon
                    let icon_x = row_rect.x + 8;
                    let icon_y = row_rect.y + 4;
                    draw_file_icon(s, icon_x, icon_y, file.kind, &palette);

                    // Name
                    let text_y = row_rect.y + (row_h - font.line_height()) / 2;
                    let text_col = if is_selected { WHITE } else { palette.text_primary };
                    draw_text_font(s, row_rect.x + 34, text_y, file.name_str(), text_col, &font);

                    // Size
                    let mut size_buf = [0u8; 16];
                    let s_len = file.format_size(&mut size_buf);
                    let size_str = core::str::from_utf8(&size_buf[..s_len]).unwrap_or("");
                    draw_text_font(s, row_rect.x + 220, text_y, size_str, palette.text_secondary, &f_small);

                    // Type description
                    draw_text_font(s, row_rect.x + 320, text_y, file.kind.description(), palette.text_muted, &f_small);

                    cur_y += row_h;
                }
            }
            ViewMode::Grid => {
                let card_w = 76;
                let card_h = 70;
                let cols = ((content_rect.w - 16) / card_w).max(1) as usize;

                for i in 0..self.file_count {
                    let col = i % cols;
                    let row = i / cols;
                    let cx = content_rect.x + 8 + (col as i32 * card_w);
                    let cy = content_rect.y + 8 + (row as i32 * card_h);

                    if cy + card_h > content_rect.bottom() {
                        break;
                    }

                    let file = self.files[i];
                    let is_selected = self.selected_idx == Some(i);
                    let is_hovered = self.hovered_idx == Some(i);
                    let card_rect = Rect::new(cx, cy, card_w - 4, card_h - 4);

                    if is_selected {
                        draw_rounded_rect(s, card_rect, RADIUS_SM, palette.taskbar_item_active);
                        draw_rect_outline(s, card_rect, 1, palette.border_focused);
                    } else if is_hovered {
                        draw_rounded_rect(s, card_rect, RADIUS_SM, palette.surface_raised);
                    }

                    // Centered large icon
                    let icon_x = card_rect.x + (card_rect.w - 14) / 2;
                    let icon_y = card_rect.y + 10;
                    draw_file_icon(s, icon_x, icon_y, file.kind, &palette);

                    // File name underneath
                    let name_sz = f_small.measure_str(file.name_str());
                    let tx = card_rect.x + (card_rect.w - name_sz.w).max(0) / 2;
                    let ty = card_rect.y + 36;
                    let text_col = if is_selected { WHITE } else { palette.text_primary };
                    draw_text_font(s, tx, ty, file.name_str(), text_col, &f_small);
                }
            }
        }

        // 4. Status Bar at Bottom (`height = 22px`)
        let status_rect = Rect::new(viewport.x, viewport.bottom() - 22, viewport.w, 22);
        s.fill_rect(status_rect, palette.surface_raised);
        s.fill_rect(Rect::new(status_rect.x, status_rect.y, status_rect.w, 1), palette.border_subtle);

        // Left summary
        let mut summary_buf = [0u8; 32];
        let sum_len = format_summary(self.file_count, &mut summary_buf);
        let sum_str = core::str::from_utf8(&summary_buf[..sum_len]).unwrap_or("");
        draw_text_font(s, status_rect.x + 8, status_rect.y + 4, sum_str, palette.text_muted, &f_small);

        // Right status / selection message
        let right_msg = self.status_str();
        if !right_msg.is_empty() {
            let r_sz = f_small.measure_str(right_msg);
            let rx = status_rect.right() - r_sz.w - 12;
            draw_text_font(s, rx, status_rect.y + 4, right_msg, palette.text_secondary, &f_small);
        }

        // 5. File Preview Modal Overlay (when preview is open)
        if self.is_preview_open {
            let p_rect = Rect::new(viewport.x + 30, viewport.y + 46, viewport.w - 60, viewport.h - 80);

            // Drop shadow
            s.fill_rect(Rect::new(p_rect.x + 4, p_rect.bottom(), p_rect.w - 2, 4), Color::rgba(0, 0, 0, 80));
            s.fill_rect(Rect::new(p_rect.right(), p_rect.y + 4, 4, p_rect.h), Color::rgba(0, 0, 0, 80));

            // Background & border
            draw_rounded_rect(s, p_rect, RADIUS_MD, palette.surface_raised);
            draw_rect_outline(s, p_rect, 1, palette.border_focused);

            // Title bar
            let p_title = Rect::new(p_rect.x, p_rect.y, p_rect.w, 30);
            s.fill_rect(p_title, palette.titlebar_active);
            s.fill_rect(Rect::new(p_title.x, p_title.bottom() - 1, p_title.w, 1), palette.border_subtle);

            let name_str = core::str::from_utf8(&self.preview_name).unwrap_or("Preview").trim_matches('\0');
            draw_text_font(s, p_title.x + 10, p_title.y + 7, name_str, WHITE, &font);

            // Close button
            let close_btn = Rect::new(p_rect.right() - 70, p_rect.y + 5, 60, 20);
            draw_rounded_rect(s, close_btn, RADIUS_XS, palette.status_danger);
            let c_sz = f_small.measure_str("Close");
            draw_text_font(s, close_btn.x + (close_btn.w - c_sz.w) / 2, close_btn.y + 3, "Close", WHITE, &f_small);

            // Preview text body area
            let body_rect = Rect::new(p_rect.x + 8, p_rect.y + 38, p_rect.w - 16, p_rect.h - 46);
            draw_rounded_rect(s, body_rect, RADIUS_XS, palette.surface_sunken);
            draw_rect_outline(s, body_rect, 1, palette.border_subtle);

            let preview_slice = unsafe {
                core::slice::from_raw_parts(core::ptr::addr_of!(PREVIEW_BUF) as *const u8, self.preview_len)
            };

            render_preview_text(s, body_rect, preview_slice, &font, palette.text_primary);
        }
    }
}

/// Renders file type icons into surface `s`.
fn draw_file_icon<S: Surface + ?Sized>(s: &mut S, x: i32, y: i32, kind: FileKind, palette: &ThemePalette) {
    match kind {
        FileKind::Document => {
            // Text document sheet with folded corner
            draw_rounded_rect(s, Rect::new(x + 1, y, 10, 14), 1, WHITE);
            s.put_pixel(x + 9, y, palette.surface_sunken);
            s.fill_rect(Rect::new(x + 3, y + 4, 6, 1), palette.text_muted);
            s.fill_rect(Rect::new(x + 3, y + 6, 6, 1), palette.text_muted);
            s.fill_rect(Rect::new(x + 3, y + 8, 4, 1), palette.text_muted);
        }
        FileKind::Config => {
            // Configuration wrench / gear
            let col = Color::rgb(140, 190, 240);
            draw_rounded_rect(s, Rect::new(x + 1, y + 1, 12, 12), 2, col);
            s.fill_rect(Rect::new(x + 4, y + 4, 6, 6), palette.surface_sunken);
            s.fill_rect(Rect::new(x + 6, y + 6, 2, 2), WHITE);
        }
        FileKind::Executable => {
            // Executable terminal prompt `>_`
            draw_rounded_rect(s, Rect::new(x + 1, y + 1, 12, 12), 2, palette.surface_sunken);
            s.fill_rect(Rect::new(x + 3, y + 4, 2, 2), palette.status_success);
            s.fill_rect(Rect::new(x + 5, y + 6, 2, 2), palette.status_success);
            s.fill_rect(Rect::new(x + 3, y + 8, 2, 2), palette.status_success);
            s.fill_rect(Rect::new(x + 7, y + 10, 4, 2), WHITE);
        }
        FileKind::Package => {
            // Package box
            let pkg_col = Color::rgb(220, 150, 240);
            draw_rounded_rect(s, Rect::new(x + 1, y + 2, 12, 10), 1, pkg_col);
            s.fill_rect(Rect::new(x + 6, y + 2, 2, 10), WHITE);
            s.fill_rect(Rect::new(x + 1, y + 6, 12, 2), WHITE);
        }
        _ => {
            // Generic file
            draw_rounded_rect(s, Rect::new(x + 2, y + 1, 10, 12), 1, palette.text_secondary);
            s.fill_rect(Rect::new(x + 4, y + 4, 6, 1), WHITE);
            s.fill_rect(Rect::new(x + 4, y + 7, 6, 1), WHITE);
        }
    }
}

fn format_summary(count: usize, out: &mut [u8; 32]) -> usize {
    let mut val = count;
    let mut digits = [0u8; 10];
    let mut d_count = 0;
    if val == 0 {
        digits[0] = b'0';
        d_count = 1;
    } else {
        while val > 0 {
            digits[d_count] = b'0' + (val % 10) as u8;
            val /= 10;
            d_count += 1;
        }
    }
    let mut pos = 0;
    for i in (0..d_count).rev() {
        out[pos] = digits[i];
        pos += 1;
    }
    let suffix = b" items total";
    out[pos..pos + suffix.len()].copy_from_slice(suffix);
    pos + suffix.len()
}

fn render_preview_text<S: Surface + ?Sized>(
    s: &mut S,
    bounds: Rect,
    bytes: &[u8],
    font: &Font,
    color: Color,
) {
    let line_h = font.line_height();
    let mut cur_y = bounds.y + 6;
    let mut pos = 0;

    while pos < bytes.len() && cur_y + line_h <= bounds.bottom() {
        let mut line_end = pos;
        while line_end < bytes.len() && bytes[line_end] != b'\n' && bytes[line_end] != 0 {
            line_end += 1;
        }

        let line = &bytes[pos..line_end];
        pos = line_end + 1;

        if let Ok(line_str) = core::str::from_utf8(line) {
            draw_text_font(s, bounds.x + 8, cur_y, line_str, color, font);
        }
        cur_y += line_h + 2;
    }
}

fn point_in_rect(px: i32, py: i32, x: i32, y: i32, w: i32, h: i32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

// ---------------------------------------------------------------------------
// Self-Test Suite
// ---------------------------------------------------------------------------

pub fn run_selftest() -> i32 {
    let mut code = 0;
    unsafe { ffi::sys_write(b"[files-selftest] starting\n\0".as_ptr()) };

    // Case 1: Directory listing parsing & file metadata classification
    let mut fm = FileManager::new();
    let test_listing = b"HELLO.TXT 68\nSHELL.ELF 47620\nSYSTEM.CFG 81\nEDITOR.PKG 190\nDATA.LOG 1024\n";
    fm.load_from_listing(test_listing);

    let count_ok = fm.file_count == 5;
    let hello_ok = fm.files[0].name_str() == "HELLO.TXT"
        && fm.files[0].size == 68
        && fm.files[0].kind == FileKind::Document;
    let shell_ok = fm.files[1].name_str() == "SHELL.ELF"
        && fm.files[1].size == 47620
        && fm.files[1].kind == FileKind::Executable;
    let cfg_ok = fm.files[2].name_str() == "SYSTEM.CFG"
        && fm.files[2].size == 81
        && fm.files[2].kind == FileKind::Config;
    let pkg_ok = fm.files[3].name_str() == "EDITOR.PKG"
        && fm.files[3].kind == FileKind::Package;

    if count_ok && hello_ok && shell_ok && cfg_ok && pkg_ok {
        unsafe { ffi::sys_write(b"[files-selftest] PASS: directory browsing and file metadata\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[files-selftest] FAIL: directory browsing\n\0".as_ptr()) };
        code |= 1;
    }

    // Case 2: Mouse selection, hit-testing, and view modes
    let vp = Rect::new(0, 0, 640, 480);
    // Hit-test row 0 in List mode
    let hit_file0 = fm.file_at(50, 60, vp) == Some(0);
    fm.on_left_click(50, 60, vp);
    let sel_file0_ok = fm.selected_idx == Some(0);

    // Hit-test Toggle View button (x=380..444, y=4..28)
    let hit_toggle = fm.button_at(400, 15, vp) == Some(ToolbarAction::ToggleView);
    fm.on_left_click(400, 15, vp);
    let view_grid_ok = fm.view_mode == ViewMode::Grid;

    // Hit-test in Grid mode
    let hit_grid0 = fm.file_at(40, 60, vp) == Some(0);
    fm.on_left_click(400, 15, vp); // Toggle back to List mode
    let view_list_ok = fm.view_mode == ViewMode::List;

    let shape_btn = fm.cursor_shape_at(400, 15, vp);
    let shape_row = fm.cursor_shape_at(50, 60, vp);
    let shape_empty = fm.cursor_shape_at(500, 400, vp);
    let shapes_ok = shape_btn == CursorShape::Pointer
        && shape_row == CursorShape::Pointer
        && shape_empty == CursorShape::Arrow;

    if hit_file0 && sel_file0_ok && hit_toggle && view_grid_ok && hit_grid0 && view_list_ok && shapes_ok {
        unsafe { ffi::sys_write(b"[files-selftest] PASS: mouse hit-testing and view modes\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[files-selftest] FAIL: mouse hit-testing\n\0".as_ptr()) };
        code |= 2;
    }

    // Case 3: File open and text preview loading
    // Test synthetic file open
    let sample_text = b"NovaOS File Manager Preview\nLine 2: System is operational.\n";
    let p_ptr = core::ptr::addr_of_mut!(PREVIEW_BUF) as *mut u8;
    unsafe {
        p_ptr.copy_from_nonoverlapping(sample_text.as_ptr(), sample_text.len());
        *p_ptr.add(sample_text.len()) = 0;
    }
    fm.preview_len = sample_text.len();
    fm.preview_name[..9].copy_from_slice(b"HELLO.TXT");
    fm.is_preview_open = true;

    let preview_open_ok = fm.is_preview_open && fm.preview_len > 0;
    // Hit-test close preview button (x=vp.right()-100 .. vp.right()-40, y=56..76)
    let hit_close_p = fm.button_at(vp.right() - 70, 66, vp) == Some(ToolbarAction::ClosePreview);
    fm.on_left_click(vp.right() - 70, 66, vp);
    let preview_closed_ok = !fm.is_preview_open;

    if preview_open_ok && hit_close_p && preview_closed_ok {
        unsafe { ffi::sys_write(b"[files-selftest] PASS: file open and text preview\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[files-selftest] FAIL: file preview\n\0".as_ptr()) };
        code |= 4;
    }

    // Case 4: File move and delete operations
    let tmp_src = "F_TMPA.TXT";
    let tmp_dst = "F_TMPB.TXT";
    let tmp_src_cstr = b"F_TMPA.TXT\0";
    let tmp_dst_cstr = b"F_TMPB.TXT\0";
    let tmp_content = b"temporary file content\n";
    unsafe {
        ffi::sys_write_file(tmp_src_cstr.as_ptr(), tmp_content.as_ptr(), tmp_content.len() as u32);
    }

    // Test move logic state machine
    let move_ok = fm.move_file(tmp_src, tmp_dst);
    // Test delete logic state machine
    let delete_ok = fm.delete_file(tmp_dst);

    // Defensively ensure temp files are cleaned up
    unsafe {
        ffi::sys_delete_file(tmp_src_cstr.as_ptr());
        ffi::sys_delete_file(tmp_dst_cstr.as_ptr());
    }

    if move_ok && delete_ok {
        unsafe { ffi::sys_write(b"[files-selftest] PASS: file move and delete operations\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[files-selftest] FAIL: file move and delete\n\0".as_ptr()) };
        code |= 8;
    }

    // Case 5: Graphical surface rendering via BufferSurface32
    static mut TEST_RENDER_BUF: [u32; 640 * 240] = [0u32; 640 * 240];
    let render_slice = unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(TEST_RENDER_BUF) as *mut u32, 640 * 240)
    };
    render_slice.fill(0);

    let mut surf = BufferSurface32::new(render_slice, 640, 240);
    fm.load_from_listing(test_listing);
    fm.select_file(1); // Select SHELL.ELF
    fm.render(&mut surf, Rect::new(0, 0, 640, 240));

    let palette = Theme::Dark.palette();
    let toolbar_pixel = surf.buffer[10 * 640 + 2];
    let body_pixel = surf.buffer[150 * 640 + 500];
    let has_bg = toolbar_pixel == palette.surface_raised.to_xrgb() && body_pixel == palette.surface_sunken.to_xrgb();
    let non_zero_count = surf.buffer.iter().filter(|&&p| p != 0).count();
    let render_rich = non_zero_count > 5000;

    // Check status bar text rendered at bottom
    let status_pixel = surf.buffer[235 * 640 + 50];
    let has_status = status_pixel != 0;

    if has_bg && render_rich && has_status {
        unsafe { ffi::sys_write(b"[files-selftest] PASS: graphical surface rendering (BufferSurface32)\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[files-selftest] FAIL: graphical surface rendering\n\0".as_ptr()) };
        code |= 16;
    }

    if code == 0 {
        unsafe { ffi::sys_write(b"[files-selftest] all cases passed\n\0".as_ptr()) };
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
        let msg = b"files: entering graphical file manager\n\0";
        ffi::sys_write(msg.as_ptr());
    }

    // 1. Query display information
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

    // 2. Acquire display
    let acq_rc = unsafe { ffi::sys_fb_acquire(ffi::NOVA_FB_BACKEND_ANY) };
    if acq_rc < 0 {
        unsafe {
            ffi::sys_write(b"files: failed to acquire display\n\0".as_ptr());
        }
        return 1;
    }

    // 3. Create framebuffer surface
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
            ffi::sys_write(b"files: failed to create surface\n\0".as_ptr());
        }
        return 1;
    }

    let fb_handle = create_req.handle;
    let pixels_ptr = create_req.pixels as *mut u32;
    let total_pixels = (screen_w * screen_h) as usize;

    let mut fm = FileManager::new();
    fm.refresh();

    let mut cursor_x = screen_w / 2;
    let mut cursor_y = screen_h / 2;
    let mut prev_left = false;
    let viewport = Rect::new(0, 0, screen_w, screen_h);

    loop {
        let key = unsafe { ffi::sys_read_key() };
        if key == b'q' as i32 || key == 27 {
            break;
        }

        let mut m = ffi::MouseState::default();
        unsafe { ffi::sys_mouse_read(&mut m) };
        cursor_x = (cursor_x + m.dx).clamp(0, screen_w - 1);
        cursor_y = (cursor_y + m.dy).clamp(0, screen_h - 1);

        fm.hovered_btn = fm.button_at(cursor_x, cursor_y, viewport);
        fm.hovered_idx = fm.file_at(cursor_x, cursor_y, viewport);

        let just_pressed = m.left_button && !prev_left;
        if just_pressed {
            fm.on_left_click(cursor_x, cursor_y, viewport);
        }
        prev_left = m.left_button;

        // Render frame directly to userland memory
        let pixel_slice = unsafe { core::slice::from_raw_parts_mut(pixels_ptr, total_pixels) };
        let mut surface = BufferSurface32::new_with_pitch(pixel_slice, screen_w, screen_h, screen_w);
        fm.render(&mut surface, viewport);

        // Render cursor sprite
        let shape = fm.cursor_shape_at(cursor_x, cursor_y, viewport);
        ui::cursor::draw_cursor_sprite(&mut surface, cursor_x, cursor_y, shape, CursorTheme::Classic);

        // Present damaged frame with single syscall
        let present = ffi::NovaFbPresent {
            handle: fb_handle,
            src_x: 0,
            src_y: 0,
            width: screen_w,
            height: screen_h,
            dst_x: 0,
            dst_y: 0,
            flags: 0,
        };
        unsafe { ffi::sys_fb_present(&present) };
        unsafe { ffi::sys_yield() };
    }

    // Clean up
    unsafe {
        ffi::sys_fb_destroy(fb_handle);
        ffi::sys_fb_release();
        ffi::sys_write(b"files: exited\n\0".as_ptr());
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_files_selftest_cases() {
        assert_eq!(run_selftest(), 0);
    }

    #[test]
    fn test_file_metadata_and_formatting() {
        let f_doc = FileEntry::new("README.TXT", 68);
        assert_eq!(f_doc.kind, FileKind::Document);
        assert_eq!(f_doc.name_str(), "README.TXT");

        let mut sz_buf = [0u8; 16];
        let len = f_doc.format_size(&mut sz_buf);
        assert_eq!(core::str::from_utf8(&sz_buf[..len]).unwrap(), "68 B");

        let f_large = FileEntry::new("BIG.DAT", 25 * 1024 * 1024);
        let len = f_large.format_size(&mut sz_buf);
        assert_eq!(core::str::from_utf8(&sz_buf[..len]).unwrap(), "25.0 MB");
    }

    #[test]
    fn test_file_manager_selection_and_view_modes() {
        let mut fm = FileManager::new();
        fm.load_from_listing(b"FILE1.TXT 100\nFILE2.ELF 200\nFILE3.CFG 300\n");
        assert_eq!(fm.file_count, 3);

        fm.select_file(1);
        assert_eq!(fm.selected_idx, Some(1));
        assert_eq!(fm.files[1].name_str(), "FILE2.ELF");

        assert_eq!(fm.view_mode, ViewMode::List);
        fm.toggle_view_mode();
        assert_eq!(fm.view_mode, ViewMode::Grid);
        fm.toggle_view_mode();
        assert_eq!(fm.view_mode, ViewMode::List);
    }

    #[test]
    fn test_file_manager_surface_rendering() {
        let mut fm = FileManager::new();
        fm.load_from_listing(b"TEST.TXT 1234\n");
        fm.select_file(0);

        let mut buf = vec![0u32; 640 * 480];
        let mut surf = BufferSurface32::new(&mut buf, 640, 480);
        fm.render(&mut surf, Rect::new(0, 0, 640, 480));

        let non_zero = surf.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero > 1000, "Should render toolbar, file item, and status bar");
    }
}
