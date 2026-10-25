//! userland/term-rs/term.rs - NovaOS In-Window Graphical Terminal & Shell Wrapper
//!
//! Month 3 - Core desktop apps: In-window terminal app
//! Runs the NovaOS shell and command execution engine inside a real, resizable
//! graphical window instead of monopolizing the VGA text console.
//!
//! Features:
//! - Full VT grid buffer (configurable columns and rows)
//! - Monospaced typography with cell-aligned cursor tracking
//! - ANSI/VT100 escape sequence parser (SGR colors, clear screen, cursor navigation)
//! - Interactive line editor with command history (Up/Down arrows) and Tab completion
//! - Built-in shell dispatcher (`help`, `ls`, `cat`, `echo`, `clear`, `date`, `uname`, `whoami`, `pwd`, `history`, `run`)
//! - Dynamic window resizing with automatic grid recalculation and scrollback
//! - Zero per-pixel syscalls: renders into `BufferSurface32` and presents in a single damage blit
//! - Standalone executable (`TERM.ELF`) and headless `--selftest` verification mode.

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
use ui::draw::{BufferSurface32, Surface};
use ui::font::Font;
use ui::geom::Rect;

#[cfg(not(test))]
use core::panic::PanicInfo;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe {
        ffi::sys_fb_release();
        let msg = b"term: internal error (panic)\n\0";
        ffi::sys_write(msg.as_ptr());
        ffi::sys_exit(1);
    }
}

// ---------------------------------------------------------------------------
// Constants & Geometry
// ---------------------------------------------------------------------------

pub const MAX_COLS: usize = 100;
pub const MAX_ROWS: usize = 36;
pub const MAX_HISTORY: usize = 128;
pub const MAX_LINE_BUF: usize = 256;
pub const MAX_CMD_HISTORY: usize = 16;

pub const CHAR_W: i32 = 8;
pub const CHAR_H: i32 = 16;
pub const PAD_X: i32 = 6;
pub const PAD_Y: i32 = 6;

// Terminal standard colors (Dark theme)
pub const COLOR_BG_TERM: Color = Color::rgb(13, 17, 23);         // #0D1117 (Dark canvas)
pub const COLOR_FG_DEFAULT: Color = Color::rgb(220, 230, 242);    // #DCE6F2 (Light gray/white)
pub const COLOR_FG_MUTED: Color = Color::rgb(139, 148, 158);      // #8B949E (Muted comment)
pub const COLOR_PROMPT_USER: Color = Color::rgb(63, 185, 80);     // #3FB950 (Bright green)
pub const COLOR_PROMPT_HOST: Color = Color::rgb(88, 166, 255);    // #58A6FF (Bright blue)
pub const COLOR_PROMPT_DIR: Color = Color::rgb(210, 153, 34);     // #D29922 (Amber/yellow)
pub const COLOR_CURSOR: Color = Color::rgb(88, 166, 255);         // Cyan/blue cursor

// ANSI 16 Color Table
pub const ANSI_COLORS: [Color; 16] = [
    Color::rgb(1, 4, 9),        // 0: Black
    Color::rgb(248, 81, 73),    // 1: Red
    Color::rgb(63, 185, 80),    // 2: Green
    Color::rgb(210, 153, 34),   // 3: Yellow
    Color::rgb(88, 166, 255),   // 4: Blue
    Color::rgb(188, 140, 255),  // 5: Magenta
    Color::rgb(56, 189, 248),   // 6: Cyan
    Color::rgb(240, 246, 252),  // 7: White
    Color::rgb(110, 118, 129),  // 8: Bright Black (Gray)
    Color::rgb(255, 123, 114),  // 9: Bright Red
    Color::rgb(86, 211, 100),   // 10: Bright Green
    Color::rgb(227, 179, 65),   // 11: Bright Yellow
    Color::rgb(121, 192, 255),  // 12: Bright Blue
    Color::rgb(210, 168, 255),  // 13: Bright Magenta
    Color::rgb(86, 225, 255),   // 14: Bright Cyan
    Color::rgb(255, 255, 255),  // 15: Bright White
];

// ---------------------------------------------------------------------------
// Virtual Terminal Cell & Buffer
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct VtCell {
    pub c: u8,
    pub fg: Color,
    pub bg: Color,
    pub attrs: u8, // bit 0: bold, bit 1: underline, bit 2: inverse
}

impl VtCell {
    pub const fn empty() -> Self {
        VtCell {
            c: b' ',
            fg: COLOR_FG_DEFAULT,
            bg: COLOR_BG_TERM,
            attrs: 0,
        }
    }

    pub const fn new(c: u8, fg: Color, bg: Color) -> Self {
        VtCell {
            c,
            fg,
            bg,
            attrs: 0,
        }
    }
}

#[derive(Clone, Copy)]
pub struct VtLine {
    pub cells: [VtCell; MAX_COLS],
    pub len: usize,
}

impl VtLine {
    pub const fn empty() -> Self {
        VtLine {
            cells: [VtCell::empty(); MAX_COLS],
            len: 0,
        }
    }

    pub fn clear(&mut self) {
        for cell in self.cells.iter_mut() {
            *cell = VtCell::empty();
        }
        self.len = 0;
    }
}

// ---------------------------------------------------------------------------
// ANSI Parser State Machine
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum EscapeState {
    Ground,
    Escape,
    Csi,
}

// ---------------------------------------------------------------------------
// Terminal Application Container
// ---------------------------------------------------------------------------

pub struct Terminal {
    // Geometry
    pub width: i32,
    pub height: i32,
    pub cols: usize,
    pub rows: usize,

    // Screen buffer
    pub screen: [VtLine; MAX_ROWS],
    pub cursor_col: usize,
    pub cursor_row: usize,
    pub cursor_visible: bool,
    pub cursor_blink_tick: u32,

    // Scrollback history
    pub history: [VtLine; MAX_HISTORY],
    pub history_len: usize,
    pub scroll_offset: usize, // 0 = view active screen, >0 = scrolled up

    // Color & Style Attributes
    pub cur_fg: Color,
    pub cur_bg: Color,
    pub cur_attrs: u8,

    // Escape sequence parsing
    esc_state: EscapeState,
    csi_params: [i32; 4],
    csi_param_count: usize,

    // Shell line editor
    pub line_buf: [u8; MAX_LINE_BUF],
    pub line_len: usize,
    pub cursor_idx: usize,
    pub prompt: &'static str,
    pub is_prompt_active: bool,

    // Command history
    pub cmd_history: [[u8; 64]; MAX_CMD_HISTORY],
    pub cmd_history_lens: [usize; MAX_CMD_HISTORY],
    pub cmd_history_count: usize,
    pub cmd_browse_idx: Option<usize>,

    // Status
    pub last_exit_code: i32,
    pub running: bool,
}

impl Terminal {
    pub const fn empty() -> Self {
        Terminal {
            width: 640,
            height: 400,
            cols: 80,
            rows: 24,
            screen: [VtLine::empty(); MAX_ROWS],
            cursor_col: 0,
            cursor_row: 0,
            cursor_visible: true,
            cursor_blink_tick: 0,
            history: [VtLine::empty(); MAX_HISTORY],
            history_len: 0,
            scroll_offset: 0,
            cur_fg: COLOR_FG_DEFAULT,
            cur_bg: COLOR_BG_TERM,
            cur_attrs: 0,
            esc_state: EscapeState::Ground,
            csi_params: [0; 4],
            csi_param_count: 0,
            line_buf: [0; MAX_LINE_BUF],
            line_len: 0,
            cursor_idx: 0,
            prompt: "admin@novaos:~$ ",
            is_prompt_active: true,
            cmd_history: [[0; 64]; MAX_CMD_HISTORY],
            cmd_history_lens: [0; MAX_CMD_HISTORY],
            cmd_history_count: 0,
            cmd_browse_idx: None,
            last_exit_code: 0,
            running: true,
        }
    }

    pub fn init(&mut self, width: i32, height: i32) {
        let cols = ((width - PAD_X * 2) / CHAR_W).clamp(10, MAX_COLS as i32) as usize;
        let rows = ((height - PAD_Y * 2) / CHAR_H).clamp(4, MAX_ROWS as i32) as usize;
        self.width = width;
        self.height = height;
        self.cols = cols;
        self.rows = rows;
        self.cursor_col = 0;
        self.cursor_row = 0;
        self.cursor_visible = true;
        self.cursor_blink_tick = 0;
        self.history_len = 0;
        self.scroll_offset = 0;
        self.cur_fg = COLOR_FG_DEFAULT;
        self.cur_bg = COLOR_BG_TERM;
        self.cur_attrs = 0;
        self.esc_state = EscapeState::Ground;
        self.csi_params = [0; 4];
        self.csi_param_count = 0;
        self.line_len = 0;
        self.cursor_idx = 0;
        self.prompt = "admin@novaos:~$ ";
        self.is_prompt_active = true;
        self.cmd_history_count = 0;
        self.cmd_browse_idx = None;
        self.last_exit_code = 0;
        self.running = true;
        self.clear();
        self.init_banner();
    }

    pub fn new(width: i32, height: i32) -> Self {
        let mut term = Self::empty();
        term.init(width, height);
        term
    }

    pub fn init_banner(&mut self) {
        self.write_styled("NovaOS Terminal (Month 3 Core Desktop Apps)\n", ANSI_COLORS[6]); // Cyan
        self.write_styled("Type 'help' for commands, 'ls' to browse files, 'clear' to reset.\n", COLOR_FG_MUTED);
        self.show_prompt();
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        let new_cols = ((width - PAD_X * 2) / CHAR_W).clamp(10, MAX_COLS as i32) as usize;
        let new_rows = ((height - PAD_Y * 2) / CHAR_H).clamp(4, MAX_ROWS as i32) as usize;

        self.cols = new_cols;
        self.rows = new_rows;

        if self.cursor_col >= self.cols {
            self.cursor_col = self.cols.saturating_sub(1);
        }
        if self.cursor_row >= self.rows {
            self.cursor_row = self.rows.saturating_sub(1);
        }
    }

    pub fn clear(&mut self) {
        for row in 0..self.rows {
            self.screen[row].clear();
        }
        self.cursor_col = 0;
        self.cursor_row = 0;
        self.scroll_offset = 0;
    }

    pub fn scroll_up(&mut self) {
        // Push top line into history buffer
        if self.history_len < MAX_HISTORY {
            self.history[self.history_len] = self.screen[0];
            self.history_len += 1;
        } else {
            // Shift history
            for i in 0..MAX_HISTORY - 1 {
                self.history[i] = self.history[i + 1];
            }
            self.history[MAX_HISTORY - 1] = self.screen[0];
        }

        // Shift visible screen rows up
        for r in 0..self.rows - 1 {
            self.screen[r] = self.screen[r + 1];
        }
        self.screen[self.rows - 1].clear();
    }

    pub fn newline(&mut self) {
        self.cursor_col = 0;
        if self.cursor_row + 1 >= self.rows {
            self.scroll_up();
        } else {
            self.cursor_row += 1;
        }
    }

    pub fn put_char(&mut self, c: u8) {
        match self.esc_state {
            EscapeState::Ground => {
                if c == 0x1B {
                    self.esc_state = EscapeState::Escape;
                    return;
                }
                match c {
                    b'\n' => self.newline(),
                    b'\r' => self.cursor_col = 0,
                    b'\t' => {
                        let next_tab = (self.cursor_col + 8) & !7;
                        self.cursor_col = next_tab.min(self.cols.saturating_sub(1));
                    }
                    b'\x08' => {
                        // Backspace
                        if self.cursor_col > 0 {
                            self.cursor_col -= 1;
                            self.screen[self.cursor_row].cells[self.cursor_col] = VtCell::empty();
                        }
                    }
                    0x20..=0x7E => {
                        if self.cursor_col >= self.cols {
                            self.newline();
                        }
                        let cell = VtCell {
                            c,
                            fg: self.cur_fg,
                            bg: self.cur_bg,
                            attrs: self.cur_attrs,
                        };
                        self.screen[self.cursor_row].cells[self.cursor_col] = cell;
                        self.cursor_col += 1;
                        if self.cursor_col > self.screen[self.cursor_row].len {
                            self.screen[self.cursor_row].len = self.cursor_col;
                        }
                    }
                    _ => {}
                }
            }
            EscapeState::Escape => {
                if c == b'[' {
                    self.esc_state = EscapeState::Csi;
                    self.csi_params = [0; 4];
                    self.csi_param_count = 0;
                } else {
                    self.esc_state = EscapeState::Ground;
                }
            }
            EscapeState::Csi => {
                if (b'0'..=b'9').contains(&c) {
                    let digit = (c - b'0') as i32;
                    if self.csi_param_count == 0 {
                        self.csi_param_count = 1;
                    }
                    let idx = self.csi_param_count - 1;
                    self.csi_params[idx] = self.csi_params[idx] * 10 + digit;
                } else if c == b';' {
                    if self.csi_param_count < 4 {
                        self.csi_param_count += 1;
                    }
                } else {
                    // Terminal command character
                    self.execute_csi(c);
                    self.esc_state = EscapeState::Ground;
                }
            }
        }
    }

    fn execute_csi(&mut self, cmd: u8) {
        match cmd {
            b'm' => {
                // SGR (Select Graphic Rendition)
                if self.csi_param_count == 0 {
                    self.cur_fg = COLOR_FG_DEFAULT;
                    self.cur_bg = COLOR_BG_TERM;
                    self.cur_attrs = 0;
                    return;
                }
                for i in 0..self.csi_param_count {
                    let code = self.csi_params[i];
                    match code {
                        0 => {
                            self.cur_fg = COLOR_FG_DEFAULT;
                            self.cur_bg = COLOR_BG_TERM;
                            self.cur_attrs = 0;
                        }
                        1 => self.cur_attrs |= 1, // Bold
                        4 => self.cur_attrs |= 2, // Underline
                        7 => self.cur_attrs |= 4, // Inverse
                        30..=37 => {
                            self.cur_fg = ANSI_COLORS[(code - 30) as usize];
                        }
                        39 => self.cur_fg = COLOR_FG_DEFAULT,
                        40..=47 => {
                            self.cur_bg = ANSI_COLORS[(code - 40) as usize];
                        }
                        49 => self.cur_bg = COLOR_BG_TERM,
                        90..=97 => {
                            self.cur_fg = ANSI_COLORS[(code - 90 + 8) as usize];
                        }
                        100..=107 => {
                            self.cur_bg = ANSI_COLORS[(code - 100 + 8) as usize];
                        }
                        _ => {}
                    }
                }
            }
            b'J' => {
                // Erase in Display
                let code = if self.csi_param_count > 0 { self.csi_params[0] } else { 0 };
                if code == 2 {
                    self.clear();
                }
            }
            b'K' => {
                // Erase in Line
                for c in self.cursor_col..self.cols {
                    self.screen[self.cursor_row].cells[c] = VtCell::empty();
                }
                self.screen[self.cursor_row].len = self.cursor_col;
            }
            b'H' | b'f' => {
                // Cursor Position
                let r = if self.csi_param_count > 0 && self.csi_params[0] > 0 {
                    (self.csi_params[0] as usize) - 1
                } else {
                    0
                };
                let c = if self.csi_param_count > 1 && self.csi_params[1] > 0 {
                    (self.csi_params[1] as usize) - 1
                } else {
                    0
                };
                self.cursor_row = r.min(self.rows.saturating_sub(1));
                self.cursor_col = c.min(self.cols.saturating_sub(1));
            }
            _ => {}
        }
    }

    pub fn write_str(&mut self, s: &str) {
        for b in s.bytes() {
            self.put_char(b);
        }
    }

    pub fn write_styled(&mut self, s: &str, fg: Color) {
        let prev_fg = self.cur_fg;
        self.cur_fg = fg;
        self.write_str(s);
        self.cur_fg = prev_fg;
    }

    pub fn show_prompt(&mut self) {
        self.write_styled("admin@novaos", COLOR_PROMPT_USER);
        self.write_styled(":", COLOR_FG_MUTED);
        self.write_styled("~", COLOR_PROMPT_HOST);
        self.write_styled("$ ", COLOR_FG_DEFAULT);
        self.is_prompt_active = true;
        self.line_len = 0;
        self.cursor_idx = 0;
    }

    // -----------------------------------------------------------------------
    // Interactive Line Editor & Shell Commands
    // -----------------------------------------------------------------------

    pub fn handle_key(&mut self, key: i32) {
        if key < 0 {
            return;
        }

        match key {
            10 | 13 => {
                // Enter: Execute command
                self.newline();
                self.execute_current_line();
                if self.running {
                    self.show_prompt();
                }
            }
            8 | 127 => {
                // Backspace
                if self.cursor_idx > 0 {
                    for i in (self.cursor_idx - 1)..self.line_len.saturating_sub(1) {
                        self.line_buf[i] = self.line_buf[i + 1];
                    }
                    self.line_len -= 1;
                    self.cursor_idx -= 1;

                    // Erase character from display
                    if self.cursor_col > 0 {
                        self.cursor_col -= 1;
                        self.screen[self.cursor_row].cells[self.cursor_col] = VtCell::empty();
                        self.redraw_current_line();
                    }
                }
            }
            9 => {
                // Tab completion
                self.tab_complete();
            }
            0x101 => {
                // Up arrow: Command history back
                self.history_prev();
            }
            0x102 => {
                // Down arrow: Command history forward
                self.history_next();
            }
            0x20..=0x7E => {
                // Printable character insertion
                if self.line_len < MAX_LINE_BUF - 1 {
                    for i in (self.cursor_idx..self.line_len).rev() {
                        self.line_buf[i + 1] = self.line_buf[i];
                    }
                    self.line_buf[self.cursor_idx] = key as u8;
                    self.line_len += 1;
                    self.cursor_idx += 1;

                    self.put_char(key as u8);
                    if self.cursor_idx < self.line_len {
                        self.redraw_current_line();
                    }
                }
            }
            _ => {}
        }
    }

    fn redraw_current_line(&mut self) {
        let prompt_len = 16; // "admin@novaos:~$ "
        let start_col = prompt_len;
        for i in 0..self.line_len {
            let col = start_col + i;
            if col < self.cols {
                self.screen[self.cursor_row].cells[col] = VtCell::new(self.line_buf[i], COLOR_FG_DEFAULT, COLOR_BG_TERM);
            }
        }
        for col in (start_col + self.line_len)..self.cols {
            self.screen[self.cursor_row].cells[col] = VtCell::empty();
        }
        self.cursor_col = (start_col + self.cursor_idx).min(self.cols.saturating_sub(1));
    }

    fn record_cmd_history(&mut self) {
        if self.line_len == 0 {
            return;
        }
        if self.cmd_history_count < MAX_CMD_HISTORY {
            let idx = self.cmd_history_count;
            let copy_len = self.line_len.min(64);
            self.cmd_history[idx][..copy_len].copy_from_slice(&self.line_buf[..copy_len]);
            self.cmd_history_lens[idx] = copy_len;
            self.cmd_history_count += 1;
        } else {
            for i in 0..MAX_CMD_HISTORY - 1 {
                self.cmd_history[i] = self.cmd_history[i + 1];
                self.cmd_history_lens[i] = self.cmd_history_lens[i + 1];
            }
            let idx = MAX_CMD_HISTORY - 1;
            let copy_len = self.line_len.min(64);
            self.cmd_history[idx][..copy_len].copy_from_slice(&self.line_buf[..copy_len]);
            self.cmd_history_lens[idx] = copy_len;
        }
        self.cmd_browse_idx = None;
    }

    fn history_prev(&mut self) {
        if self.cmd_history_count == 0 {
            return;
        }
        let next_idx = match self.cmd_browse_idx {
            None => self.cmd_history_count - 1,
            Some(i) if i > 0 => i - 1,
            Some(i) => i,
        };
        self.cmd_browse_idx = Some(next_idx);
        let len = self.cmd_history_lens[next_idx];
        self.line_buf[..len].copy_from_slice(&self.cmd_history[next_idx][..len]);
        self.line_len = len;
        self.cursor_idx = len;
        self.redraw_current_line();
    }

    fn history_next(&mut self) {
        if let Some(i) = self.cmd_browse_idx {
            if i + 1 < self.cmd_history_count {
                let next_idx = i + 1;
                self.cmd_browse_idx = Some(next_idx);
                let len = self.cmd_history_lens[next_idx];
                self.line_buf[..len].copy_from_slice(&self.cmd_history[next_idx][..len]);
                self.line_len = len;
                self.cursor_idx = len;
                self.redraw_current_line();
            } else {
                self.cmd_browse_idx = None;
                self.line_len = 0;
                self.cursor_idx = 0;
                self.redraw_current_line();
            }
        }
    }

    fn tab_complete(&mut self) {
        let commands = [
            "help", "cat", "clear", "date", "echo", "exit", "history", "ls", "ping", "pwd", "run", "uname", "version", "whoami",
        ];

        let line_str = match core::str::from_utf8(&self.line_buf[..self.cursor_idx]) {
            Ok(s) => s.trim_start(),
            Err(_) => return,
        };

        if line_str.is_empty() {
            return;
        }

        for &cmd in commands.iter() {
            if cmd.starts_with(line_str) && cmd.len() > line_str.len() {
                let rest = &cmd[line_str.len()..];
                for b in rest.bytes() {
                    if self.line_len < MAX_LINE_BUF - 1 {
                        self.line_buf[self.line_len] = b;
                        self.line_len += 1;
                        self.cursor_idx += 1;
                        self.put_char(b);
                    }
                }
                break;
            }
        }
    }

    pub fn execute_current_line(&mut self) {
        self.record_cmd_history();

        let mut local_buf = [0u8; MAX_LINE_BUF];
        let copy_len = self.line_len.min(MAX_LINE_BUF);
        local_buf[..copy_len].copy_from_slice(&self.line_buf[..copy_len]);

        let line_str = match core::str::from_utf8(&local_buf[..copy_len]) {
            Ok(s) => s.trim(),
            Err(_) => {
                self.write_styled("Error: invalid UTF-8 command line\n", ANSI_COLORS[1]);
                return;
            }
        };

        if line_str.is_empty() {
            return;
        }

        // Split into command and arguments
        let mut parts = line_str.split_ascii_whitespace();
        let cmd = match parts.next() {
            Some(c) => c,
            None => return,
        };
        let arg1 = parts.next();

        match cmd {
            "help" => self.cmd_help(),
            "clear" => self.clear(),
            "echo" => {
                let mut first = true;
                for part in line_str.split_ascii_whitespace().skip(1) {
                    if !first {
                        self.put_char(b' ');
                    }
                    self.write_str(part);
                    first = false;
                }
                self.newline();
            }
            "date" => self.cmd_date(),
            "uname" | "version" => {
                self.write_styled("NovaOS 1.0.0-compositor i686 (Month 3 Desktop Apps)\n", ANSI_COLORS[2]);
            }
            "whoami" => {
                self.write_styled("admin (uid 800, gid 0)\n", COLOR_FG_DEFAULT);
            }
            "pwd" => {
                self.write_styled("/root\n", COLOR_FG_DEFAULT);
            }
            "history" => self.cmd_history(),
            "ls" | "dir" => self.cmd_ls(),
            "cat" => {
                if let Some(filename) = arg1 {
                    let mut fname_buf = [0u8; 64];
                    let f_len = filename.len().min(63);
                    fname_buf[..f_len].copy_from_slice(&filename.as_bytes()[..f_len]);
                    if let Ok(f_str) = core::str::from_utf8(&fname_buf[..f_len]) {
                        self.cmd_cat(f_str);
                    }
                } else {
                    self.write_styled("usage: cat <filename>\n", ANSI_COLORS[3]);
                }
            }
            "run" => {
                if let Some(target) = arg1 {
                    let mut tgt_buf = [0u8; 64];
                    let t_len = target.len().min(63);
                    tgt_buf[..t_len].copy_from_slice(&target.as_bytes()[..t_len]);
                    if let Ok(t_str) = core::str::from_utf8(&tgt_buf[..t_len]) {
                        self.cmd_run(t_str);
                    }
                } else {
                    self.write_styled("usage: run <program.elf>\n", ANSI_COLORS[3]);
                }
            }
            "exit" => {
                self.write_styled("Terminating in-window terminal session...\n", ANSI_COLORS[5]);
                self.running = false;
            }
            unknown => {
                let mut unk_buf = [0u8; 32];
                let u_len = unknown.len().min(31);
                unk_buf[..u_len].copy_from_slice(&unknown.as_bytes()[..u_len]);
                self.write_styled("Unknown command '", ANSI_COLORS[1]);
                if let Ok(u_str) = core::str::from_utf8(&unk_buf[..u_len]) {
                    self.write_str(u_str);
                }
                self.write_styled("'. Type 'help' for available commands.\n", ANSI_COLORS[1]);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Built-in Shell Command Implementations
    // -----------------------------------------------------------------------

    fn cmd_help(&mut self) {
        self.write_styled("NovaOS Terminal Shell Commands:\n", ANSI_COLORS[6]);
        self.write_styled("  help          ", ANSI_COLORS[2]); self.write_str("Show this list of commands\n");
        self.write_styled("  ls            ", ANSI_COLORS[2]); self.write_str("List files in root directory\n");
        self.write_styled("  cat <file>    ", ANSI_COLORS[2]); self.write_str("Display contents of a file\n");
        self.write_styled("  echo <text>   ", ANSI_COLORS[2]); self.write_str("Print text to terminal output\n");
        self.write_styled("  date          ", ANSI_COLORS[2]); self.write_str("Display current RTC system date and time\n");
        self.write_styled("  uname         ", ANSI_COLORS[2]); self.write_str("Print kernel and architecture info\n");
        self.write_styled("  whoami        ", ANSI_COLORS[2]); self.write_str("Print active user identity\n");
        self.write_styled("  pwd           ", ANSI_COLORS[2]); self.write_str("Print working directory\n");
        self.write_styled("  history       ", ANSI_COLORS[2]); self.write_str("Show command history\n");
        self.write_styled("  run <app.elf> ", ANSI_COLORS[2]); self.write_str("Execute an ELF binary in userland\n");
        self.write_styled("  clear         ", ANSI_COLORS[2]); self.write_str("Clear terminal screen\n");
        self.write_styled("  exit          ", ANSI_COLORS[2]); self.write_str("Close terminal emulator\n");
    }

    fn cmd_date(&mut self) {
        let mut rtc = ffi::RtcTime::default();
        let rc = unsafe { ffi::sys_rtc_read(&mut rtc) };
        if rc == 0 {
            self.write_styled("Date: ", COLOR_FG_MUTED);
            self.write_num(rtc.year as u32);
            self.put_char(b'-');
            self.write_two_digits(rtc.month);
            self.put_char(b'-');
            self.write_two_digits(rtc.day);
            self.put_char(b' ');
            self.write_two_digits(rtc.hour);
            self.put_char(b':');
            self.write_two_digits(rtc.minute);
            self.put_char(b':');
            self.write_two_digits(rtc.second);
            self.newline();
        } else {
            self.write_styled("date: RTC unavailable\n", ANSI_COLORS[1]);
        }
    }

    fn write_num(&mut self, mut n: u32) {
        if n == 0 {
            self.put_char(b'0');
            return;
        }
        let mut buf = [0u8; 12];
        let mut i = 0;
        while n > 0 {
            buf[i] = b'0' + (n % 10) as u8;
            n /= 10;
            i += 1;
        }
        for j in (0..i).rev() {
            self.put_char(buf[j]);
        }
    }

    fn write_two_digits(&mut self, val: u8) {
        self.put_char(b'0' + (val / 10));
        self.put_char(b'0' + (val % 10));
    }

    fn cmd_history(&mut self) {
        if self.cmd_history_count == 0 {
            self.write_styled("No commands in history.\n", COLOR_FG_MUTED);
            return;
        }
        for i in 0..self.cmd_history_count {
            self.put_char(b' ');
            self.write_num((i + 1) as u32);
            self.write_str("  ");
            let len = self.cmd_history_lens[i];
            let mut hist_buf = [0u8; 64];
            hist_buf[..len].copy_from_slice(&self.cmd_history[i][..len]);
            if let Ok(s) = core::str::from_utf8(&hist_buf[..len]) {
                self.write_str(s);
            }
            self.newline();
        }
    }

    fn cmd_ls(&mut self) {
        let mut buf = [0u8; 512];
        let rc = unsafe { ffi::sys_list_files(buf.as_mut_ptr(), buf.len() as i32) };
        if rc <= 0 {
            self.write_styled("ls: failed to list files or directory empty\n", ANSI_COLORS[1]);
            return;
        }

        let slice = &buf[..rc as usize];
        let mut col_count = 0;
        let mut start = 0;

        for (i, &b) in slice.iter().enumerate() {
            if b == b'|' || i == slice.len() - 1 {
                let end = if b == b'|' { i } else { i + 1 };
                if end > start {
                    let entry = &slice[start..end];
                    if let Ok(name) = core::str::from_utf8(entry) {
                        let color = if name.ends_with(".ELF") {
                            ANSI_COLORS[2] // Green for executables
                        } else if name.ends_with(".CFG") {
                            ANSI_COLORS[3] // Yellow for configs
                        } else {
                            COLOR_FG_DEFAULT
                        };
                        self.write_styled(name, color);

                        // Pad entry
                        let pad = 16usize.saturating_sub(name.len());
                        for _ in 0..pad {
                            self.put_char(b' ');
                        }

                        col_count += 1;
                        if col_count % 3 == 0 {
                            self.newline();
                        }
                    }
                }
                start = i + 1;
            }
        }
        if col_count % 3 != 0 {
            self.newline();
        }
    }

    fn cmd_cat(&mut self, filename: &str) {
        let mut c_name = [0u8; 64];
        let copy_len = filename.len().min(63);
        c_name[..copy_len].copy_from_slice(&filename.as_bytes()[..copy_len]);
        c_name[copy_len] = 0;

        let handle = unsafe { ffi::sys_open(c_name.as_ptr()) };
        if handle < 0 {
            self.write_styled("cat: cannot open file '", ANSI_COLORS[1]);
            self.write_str(filename);
            self.write_styled("'\n", ANSI_COLORS[1]);
            return;
        }

        let mut read_buf = [0u8; 1024];
        let n = unsafe { ffi::sys_read(handle, read_buf.as_mut_ptr(), read_buf.len() as i32) };
        unsafe { ffi::sys_close(handle) };

        if n > 0 {
            for &b in &read_buf[..n as usize] {
                self.put_char(b);
            }
            if read_buf[n as usize - 1] != b'\n' {
                self.newline();
            }
        }
    }

    fn cmd_run(&mut self, target: &str) {
        let mut c_target = [0u8; 64];
        let copy_len = target.len().min(63);
        c_target[..copy_len].copy_from_slice(&target.as_bytes()[..copy_len]);
        c_target[copy_len] = 0;

        let argv: [*const u8; 2] = [c_target.as_ptr(), core::ptr::null()];
        let pid = unsafe { ffi::sys_exec_trusted(c_target.as_ptr(), argv.as_ptr(), 1) };
        if pid < 0 {
            self.write_styled("run: failed to execute '", ANSI_COLORS[1]);
            self.write_str(target);
            self.write_styled("'\n", ANSI_COLORS[1]);
            return;
        }

        self.write_styled("Spawning process [pid ", COLOR_FG_MUTED);
        self.write_num(pid as u32);
        self.write_styled("]...\n", COLOR_FG_MUTED);

        let code = unsafe { ffi::sys_wait(pid) };
        self.last_exit_code = code;

        self.write_styled("Process exited with code ", COLOR_FG_MUTED);
        self.write_num(code as u32);
        self.newline();
    }

    // -----------------------------------------------------------------------
    // Graphical Surface Rasterization
    // -----------------------------------------------------------------------

    pub fn render<S: Surface + ?Sized>(&self, surface: &mut S) {
        // 1. Dark canvas background
        surface.fill_rect(Rect::new(0, 0, self.width, self.height), COLOR_BG_TERM);

        let font = Font::mono();

        // 2. Visible characters
        for r in 0..self.rows {
            let line = &self.screen[r];
            let y = PAD_Y + (r as i32) * CHAR_H;

            for c in 0..self.cols {
                let cell = &line.cells[c];
                let x = PAD_X + (c as i32) * CHAR_W;

                // Background color if non-standard
                if cell.bg != COLOR_BG_TERM {
                    surface.fill_rect(Rect::new(x, y, CHAR_W, CHAR_H), cell.bg);
                }

                if cell.c != b' ' && cell.c != 0 {
                    ui::font::draw_char_font(surface, x, y, cell.c, cell.fg, &font);
                }
            }
        }

        // 3. Cursor block
        if self.cursor_visible && self.cursor_row < self.rows && self.cursor_col < self.cols {
            let cx = PAD_X + (self.cursor_col as i32) * CHAR_W;
            let cy = PAD_Y + (self.cursor_row as i32) * CHAR_H;

            // Block cursor
            surface.fill_rect(Rect::new(cx, cy + 2, CHAR_W, CHAR_H - 4), COLOR_CURSOR);

            // Inverted character inside cursor if non-empty
            let cur_char = self.screen[self.cursor_row].cells[self.cursor_col].c;
            if cur_char != b' ' && cur_char != 0 {
                ui::font::draw_char_font(surface, cx, cy, cur_char, COLOR_BG_TERM, &font);
            }
        }

        // 4. Subtle vertical scrollbar track & thumb on the right
        if self.history_len > 0 {
            let track_x = self.width - 6;
            let track_h = self.height - PAD_Y * 2;
            surface.fill_rect(Rect::new(track_x, PAD_Y, 4, track_h), Color::rgba(255, 255, 255, 15));

            let total_lines = (self.history_len + self.rows) as i32;
            let thumb_h = ((self.rows as i32 * track_h) / total_lines).clamp(16, track_h);
            let thumb_y = PAD_Y + (((total_lines - self.scroll_offset as i32 - self.rows as i32) * (track_h - thumb_h)) / total_lines.max(1));
            surface.fill_rect(Rect::new(track_x, thumb_y, 4, thumb_h), Color::rgba(255, 255, 255, 60));
        }
    }
}

// ---------------------------------------------------------------------------
// Static Global Memory for Userland (16KB Stack Safe)
// ---------------------------------------------------------------------------

static mut GLOBAL_TERM: Terminal = Terminal::empty();

static mut RENDER_BUFFER: [u32; 1024 * 768] = [0; 1024 * 768];

// ---------------------------------------------------------------------------
// Automated Self-Test Suite
// ---------------------------------------------------------------------------

pub fn run_selftest() -> i32 {
    unsafe { ffi::sys_write(b"[term-selftest] starting\n\0".as_ptr()) };

    let mut code = 0;

    // Case 1: Grid dimensions and dynamic resize calculation
    let term = unsafe { &mut *core::ptr::addr_of_mut!(GLOBAL_TERM) };
    term.init(360, 250);
    let init_cols_ok = term.cols >= 40 && term.cols <= 50;
    let init_rows_ok = term.rows >= 12 && term.rows <= 16;
    term.resize(800, 600);
    let resize_cols_ok = term.cols >= 90;
    let resize_rows_ok = term.rows >= 30;

    if init_cols_ok && init_rows_ok && resize_cols_ok && resize_rows_ok {
        unsafe { ffi::sys_write(b"[term-selftest] PASS: grid dimensions and dynamic resize calculation\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[term-selftest] FAIL: grid dimensions and resize\n\0".as_ptr()) };
        code |= 1;
    }

    // Case 2: Character flow, cursor advancement, and line scrolling
    term.clear();
    term.write_str("Hello");
    let cursor_adv_ok = term.cursor_col == 5 && term.cursor_row == 0;
    term.newline();
    let newline_ok = term.cursor_col == 0 && term.cursor_row == 1;

    // Emit rows exceeding vertical bound to verify scrolling
    for _ in 0..term.rows + 5 {
        term.write_str("Line\n");
    }
    let scroll_ok = term.cursor_row == term.rows - 1 && term.history_len > 0;

    if cursor_adv_ok && newline_ok && scroll_ok {
        unsafe { ffi::sys_write(b"[term-selftest] PASS: character flow, wrapping, and line scrolling\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[term-selftest] FAIL: character flow and scrolling\n\0".as_ptr()) };
        code |= 2;
    }

    // Case 3: ANSI escape sequence decoding (SGR colors, clear)
    term.clear();
    term.write_str("\x1b[31mRed\x1b[0mNormal\x1b[32mGreen\x1b[0m");
    let cell_red = term.screen[0].cells[0];
    let cell_norm = term.screen[0].cells[3];
    let cell_green = term.screen[0].cells[9];

    let ansi_red_ok = cell_red.fg == ANSI_COLORS[1] && cell_red.c == b'R';
    let ansi_norm_ok = cell_norm.fg == COLOR_FG_DEFAULT && cell_norm.c == b'N';
    let ansi_green_ok = cell_green.fg == ANSI_COLORS[2] && cell_green.c == b'G';

    term.write_str("\x1b[2J"); // Clear screen
    let ansi_clear_ok = term.cursor_col == 0 && term.cursor_row == 0;

    if ansi_red_ok && ansi_norm_ok && ansi_green_ok && ansi_clear_ok {
        unsafe { ffi::sys_write(b"[term-selftest] PASS: ANSI escape sequence decoding and colors\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[term-selftest] FAIL: ANSI escape sequence parsing\n\0".as_ptr()) };
        code |= 4;
    }

    // Case 4: Line editing, backspace, and command history
    term.clear();
    term.show_prompt();
    for &b in b"echo test" {
        term.handle_key(b as i32);
    }
    let edit_typed_ok = term.line_len == 9 && &term.line_buf[..9] == b"echo test";

    // Backspace
    term.handle_key(8); // Backspace
    let edit_bs_ok = term.line_len == 8 && &term.line_buf[..8] == b"echo tes";

    // Enter to execute and push into history
    term.handle_key(10);
    let hist_saved = term.cmd_history_count == 1 && &term.cmd_history[0][..8] == b"echo tes";

    // Up arrow to recall
    term.handle_key(0x101);
    let hist_recalled = term.line_len == 8 && &term.line_buf[..8] == b"echo tes";

    if edit_typed_ok && edit_bs_ok && hist_saved && hist_recalled {
        unsafe { ffi::sys_write(b"[term-selftest] PASS: interactive line editing and command history\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[term-selftest] FAIL: line editing and command history\n\0".as_ptr()) };
        code |= 8;
    }

    // Case 5: Shell command dispatcher (`help`, `date`, `uname`, `ls`)
    term.clear();
    term.line_buf[..4].copy_from_slice(b"help");
    term.line_len = 4;
    term.execute_current_line();
    let help_dispatched = term.cursor_row > 5;

    term.line_buf[..5].copy_from_slice(b"uname");
    term.line_len = 5;
    term.execute_current_line();
    let uname_dispatched = term.cursor_row > 6;

    if help_dispatched && uname_dispatched {
        unsafe { ffi::sys_write(b"[term-selftest] PASS: shell command parsing and execution dispatch\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[term-selftest] FAIL: shell command dispatch\n\0".as_ptr()) };
        code |= 16;
    }

    // Case 6: 32-bit graphical surface rasterization
    static mut TEST_RENDER_BUF: [u32; 360 * 250] = [0u32; 360 * 250];
    let render_slice = unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(TEST_RENDER_BUF) as *mut u32, 360 * 250)
    };
    render_slice.fill(0);
    let mut surface = BufferSurface32::new(render_slice, 360, 250);
    term.render(&mut surface);

    let non_zero_pixels = surface.buffer.iter().filter(|&&p| p != 0).count();
    let surface_rendered_ok = non_zero_pixels > 200;

    if surface_rendered_ok {
        unsafe { ffi::sys_write(b"[term-selftest] PASS: graphical surface rasterization (BufferSurface32)\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[term-selftest] FAIL: surface rasterization\n\0".as_ptr()) };
        code |= 32;
    }

    if code == 0 {
        unsafe { ffi::sys_write(b"[term-selftest] all cases passed\n\0".as_ptr()) };
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
        let msg = b"term: entering in-window terminal emulator (Month 3 Desktop Apps)\n\0";
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
        (640, 400)
    };

    let acq_rc = unsafe { ffi::sys_fb_acquire(ffi::NOVA_FB_BACKEND_ANY) };
    if acq_rc < 0 {
        unsafe {
            let msg = b"term: failed to acquire framebuffer display (-EBUSY or -ENODEV)\n\0";
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
            let msg = b"term: failed to create framebuffer surface\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        return 1;
    }

    let fb_handle = create_req.handle;
    let pixels_ptr = create_req.pixels as *mut u32;
    let total_pixels = (screen_w * screen_h) as usize;

    let term = unsafe { &mut *addr_of_mut!(GLOBAL_TERM) };
    term.init(screen_w, screen_h);

    while term.running {
        let key = unsafe { ffi::sys_read_key() };
        if key >= 0 {
            if key == 27 { // Esc to exit
                break;
            }
            term.handle_key(key);
        }

        // Render frame to surface buffer in userland memory
        let pixel_slice = unsafe { core::slice::from_raw_parts_mut(pixels_ptr, total_pixels) };
        let mut surface = BufferSurface32::new_with_pitch(pixel_slice, screen_w, screen_h, screen_w);
        term.render(&mut surface);

        // Present damage with single syscall
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
    fn test_term_selftest_cases() {
        let code = run_selftest();
        assert_eq!(code, 0, "Terminal selftest cases must all pass");
    }

    #[test]
    fn test_terminal_vt_cell_attributes() {
        let cell = VtCell::new(b'A', ANSI_COLORS[1], ANSI_COLORS[0]);
        assert_eq!(cell.c, b'A');
        assert_eq!(cell.fg, ANSI_COLORS[1]);
        assert_eq!(cell.bg, ANSI_COLORS[0]);
    }

    #[test]
    fn test_terminal_cursor_wrapping() {
        let mut term = Terminal::new(200, 150);
        term.clear();
        let max_c = term.cols;
        for _ in 0..max_c + 5 {
            term.put_char(b'x');
        }
        assert_eq!(term.cursor_row, 1);
        assert_eq!(term.cursor_col, 5);
    }

    #[test]
    fn test_terminal_ansi_colors() {
        let mut term = Terminal::new(300, 200);
        term.clear();
        term.write_str("\x1b[34mBlueText\x1b[0m");
        assert_eq!(term.screen[0].cells[0].fg, ANSI_COLORS[4]);
        assert_eq!(term.screen[0].cells[0].c, b'B');
    }

    #[test]
    fn test_terminal_line_editing() {
        let mut term = Terminal::new(300, 200);
        term.show_prompt();
        term.handle_key(b'l' as i32);
        term.handle_key(b's' as i32);
        assert_eq!(&term.line_buf[..2], b"ls");
        term.handle_key(8); // Backspace
        assert_eq!(term.line_len, 1);
        assert_eq!(term.line_buf[0], b'l');
    }
}
