//! userland/desktop/apps/nova-term/src/ansi.rs
//!
//! ANSI/VT100/xterm Escape Sequence Parser with 24-bit TrueColor for NovaTerm.

use crate::grid::*;
use crate::theme::ColorScheme;
use nova_ui::prelude::Color;

#[cfg(not(test))]
use alloc::{string::String, vec::Vec};

#[cfg(test)]
use std::{string::String, vec::Vec};

/// Internal state machine modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParserState {
    Ground,
    Escape,
    CsiEntry,
    CsiParam,
    OscString,
}

/// Comprehensive ANSI escape sequence processor.
pub struct AnsiParser {
    state: ParserState,
    params: Vec<u32>,
    current_param: u32,
    has_param: bool,
    is_private_mode: bool, // '?' prefix (e.g. ?25h, ?1049h)
    osc_buffer: Vec<u8>,
    pub window_title: String,
}

impl AnsiParser {
    pub fn new() -> Self {
        Self {
            state: ParserState::Ground,
            params: Vec::new(),
            current_param: 0,
            has_param: false,
            is_private_mode: false,
            osc_buffer: Vec::new(),
            window_title: String::new(),
        }
    }

    /// Feeds a byte slice into the parser, modifying grid state.
    pub fn parse_bytes(&mut self, bytes: &[u8], grid: &mut TerminalGrid, theme: &ColorScheme) {
        for &b in bytes {
            self.parse_byte(b, grid, theme);
        }
    }

    /// Feeds a single byte into the ANSI state machine.
    pub fn parse_byte(&mut self, b: u8, grid: &mut TerminalGrid, theme: &ColorScheme) {
        match self.state {
            ParserState::Ground => match b {
                0x1B => {
                    self.state = ParserState::Escape;
                }
                b'\r' => grid.carriage_return(),
                b'\n' => grid.line_feed(),
                b'\t' => grid.tab(),
                0x08 => grid.backspace(),
                0x07 => {} // Bell (audio chime trigger)
                0x20..=0x7E => {
                    grid.put_char(b as char);
                }
                _ => {
                    // Non-ASCII UTF-8 or raw byte
                    if b >= 0x80 {
                        grid.put_char(b as char);
                    }
                }
            },
            ParserState::Escape => match b {
                b'[' => {
                    self.state = ParserState::CsiEntry;
                    self.params.clear();
                    self.current_param = 0;
                    self.has_param = false;
                    self.is_private_mode = false;
                }
                b']' => {
                    self.state = ParserState::OscString;
                    self.osc_buffer.clear();
                }
                b'7' => {
                    grid.save_cursor();
                    self.state = ParserState::Ground;
                }
                b'8' => {
                    grid.restore_cursor();
                    self.state = ParserState::Ground;
                }
                b'M' => {
                    // Reverse Index (scroll down if at top)
                    if grid.cursor_y > grid.scroll_top {
                        grid.cursor_y -= 1;
                    } else {
                        grid.scroll_down(1);
                    }
                    self.state = ParserState::Ground;
                }
                _ => {
                    self.state = ParserState::Ground;
                }
            },
            ParserState::CsiEntry => {
                if b == b'?' {
                    self.is_private_mode = true;
                    self.state = ParserState::CsiParam;
                } else if b.is_ascii_digit() {
                    self.current_param = (b - b'0') as u32;
                    self.has_param = true;
                    self.state = ParserState::CsiParam;
                } else if b == b';' {
                    self.params.push(0);
                    self.state = ParserState::CsiParam;
                } else {
                    self.execute_csi(b, grid, theme);
                    self.state = ParserState::Ground;
                }
            }
            ParserState::CsiParam => {
                if b.is_ascii_digit() {
                    self.current_param = self.current_param.saturating_mul(10).saturating_add((b - b'0') as u32);
                    self.has_param = true;
                } else if b == b';' {
                    let val = if self.has_param { self.current_param } else { 0 };
                    self.params.push(val);
                    self.current_param = 0;
                    self.has_param = false;
                } else {
                    if self.has_param {
                        self.params.push(self.current_param);
                    }
                    self.execute_csi(b, grid, theme);
                    self.state = ParserState::Ground;
                }
            }
            ParserState::OscString => {
                if b == 0x07 || b == 0x1B {
                    // Terminate OSC string and extract title
                    if self.osc_buffer.starts_with(b"0;") || self.osc_buffer.starts_with(b"2;") {
                        if let Ok(title) = core::str::from_utf8(&self.osc_buffer[2..]) {
                            self.window_title = title.into();
                        }
                    }
                    self.state = ParserState::Ground;
                } else {
                    self.osc_buffer.push(b);
                }
            }
        }
    }

    /// Dispatches parsed CSI sequence command.
    fn execute_csi(&mut self, cmd: u8, grid: &mut TerminalGrid, theme: &ColorScheme) {
        let p1 = self.params.first().copied().unwrap_or(0);
        let p2 = self.params.get(1).copied().unwrap_or(0);

        match cmd {
            // Cursor Up
            b'A' => {
                let count = if p1 == 0 { 1 } else { p1 } as usize;
                grid.cursor_y = grid.cursor_y.saturating_sub(count).max(grid.scroll_top);
            }
            // Cursor Down
            b'B' => {
                let count = if p1 == 0 { 1 } else { p1 } as usize;
                grid.cursor_y = (grid.cursor_y + count).min(grid.scroll_bottom);
            }
            // Cursor Forward
            b'C' => {
                let count = if p1 == 0 { 1 } else { p1 } as usize;
                grid.cursor_x = (grid.cursor_x + count).min(grid.cols.saturating_sub(1));
            }
            // Cursor Back
            b'D' => {
                let count = if p1 == 0 { 1 } else { p1 } as usize;
                grid.cursor_x = grid.cursor_x.saturating_sub(count);
            }
            // Cursor Next Line
            b'E' => {
                let count = if p1 == 0 { 1 } else { p1 } as usize;
                grid.cursor_y = (grid.cursor_y + count).min(grid.scroll_bottom);
                grid.cursor_x = 0;
            }
            // Cursor Previous Line
            b'F' => {
                let count = if p1 == 0 { 1 } else { p1 } as usize;
                grid.cursor_y = grid.cursor_y.saturating_sub(count).max(grid.scroll_top);
                grid.cursor_x = 0;
            }
            // Cursor Horizontal Absolute
            b'G' => {
                let col = if p1 == 0 { 1 } else { p1 } as usize;
                grid.cursor_x = col.saturating_sub(1).min(grid.cols.saturating_sub(1));
            }
            // Cursor Position (1-indexed row and col)
            b'H' | b'f' => {
                let r = if p1 == 0 { 1 } else { p1 } as usize;
                let c = if p2 == 0 { 1 } else { p2 } as usize;
                grid.set_cursor(c.saturating_sub(1), r.saturating_sub(1));
            }
            // Line Position Absolute
            b'd' => {
                let r = if p1 == 0 { 1 } else { p1 } as usize;
                grid.cursor_y = r.saturating_sub(1).min(grid.rows.saturating_sub(1));
            }
            // Erase in Display
            b'J' => {
                grid.clear_screen(p1);
            }
            // Erase in Line
            b'K' => {
                grid.clear_line(p1);
            }
            // Scroll Up
            b'S' => {
                let count = if p1 == 0 { 1 } else { p1 } as usize;
                grid.scroll_up(count);
            }
            // Scroll Down
            b'T' => {
                let count = if p1 == 0 { 1 } else { p1 } as usize;
                grid.scroll_down(count);
            }
            // Set Scroll Margins (DECSTBM)
            b'r' => {
                let top = if p1 == 0 { 1 } else { p1 } as usize;
                let bottom = if p2 == 0 { grid.rows } else { p2 as usize };
                if top < bottom && bottom <= grid.rows {
                    grid.scroll_top = top - 1;
                    grid.scroll_bottom = bottom - 1;
                    grid.cursor_x = 0;
                    grid.cursor_y = grid.scroll_top;
                }
            }
            // Save Cursor (ANSI.SYS)
            b's' => {
                grid.save_cursor();
            }
            // Restore Cursor (ANSI.SYS)
            b'u' => {
                grid.restore_cursor();
            }
            // Select Graphic Rendition (SGR) with 24-bit TrueColor
            b'm' => {
                self.execute_sgr(grid, theme);
            }
            // Mode Set / Reset
            b'h' | b'l' => {
                let enable = cmd == b'h';
                if self.is_private_mode {
                    match p1 {
                        25 => grid.cursor_visible = enable,
                        1049 => grid.set_alt_screen(enable),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// Executes SGR formatting attributes and 24-bit TrueColor decoding.
    fn execute_sgr(&self, grid: &mut TerminalGrid, theme: &ColorScheme) {
        if self.params.is_empty() {
            // Default reset
            grid.current_fg = theme.foreground;
            grid.current_bg = theme.background;
            grid.current_flags = 0;
            return;
        }

        let mut idx = 0;
        while idx < self.params.len() {
            let code = self.params[idx];
            match code {
                0 => {
                    grid.current_fg = theme.foreground;
                    grid.current_bg = theme.background;
                    grid.current_flags = 0;
                }
                1 => grid.current_flags |= FLAG_BOLD,
                2 => grid.current_flags |= FLAG_DIM,
                3 => grid.current_flags |= FLAG_ITALIC,
                4 => grid.current_flags |= FLAG_UNDERLINE,
                5 => grid.current_flags |= FLAG_BLINK,
                7 => grid.current_flags |= FLAG_INVERSE,
                9 => grid.current_flags |= FLAG_STRIKETHROUGH,
                22 => grid.current_flags &= !(FLAG_BOLD | FLAG_DIM),
                24 => grid.current_flags &= !FLAG_UNDERLINE,
                27 => grid.current_flags &= !FLAG_INVERSE,
                // Standard 8 ANSI Foregrounds
                30..=37 => {
                    let color_idx = (code - 30) as usize;
                    grid.current_fg = theme.ansi[color_idx];
                }
                // Extended Foreground (256-color or 24-bit TrueColor)
                38 => {
                    if idx + 1 < self.params.len() {
                        let sub_mode = self.params[idx + 1];
                        if sub_mode == 5 && idx + 2 < self.params.len() {
                            // 256-color: \x1b[38;5;<n>m
                            let color_256 = self.params[idx + 2] as u8;
                            grid.current_fg = theme.resolve_256_color(color_256);
                            idx += 2;
                        } else if sub_mode == 2 && idx + 4 < self.params.len() {
                            // 24-bit TrueColor: \x1b[38;2;<r>;<g>;<b>m
                            let r = self.params[idx + 2] as u8;
                            let g = self.params[idx + 3] as u8;
                            let b = self.params[idx + 4] as u8;
                            grid.current_fg = Color::rgba(r, g, b, 255);
                            idx += 4;
                        }
                    }
                }
                // Default Foreground
                39 => grid.current_fg = theme.foreground,
                // Standard 8 ANSI Backgrounds
                40..=47 => {
                    let color_idx = (code - 40) as usize;
                    grid.current_bg = theme.ansi[color_idx];
                }
                // Extended Background (256-color or 24-bit TrueColor)
                48 => {
                    if idx + 1 < self.params.len() {
                        let sub_mode = self.params[idx + 1];
                        if sub_mode == 5 && idx + 2 < self.params.len() {
                            // 256-color background
                            let color_256 = self.params[idx + 2] as u8;
                            grid.current_bg = theme.resolve_256_color(color_256);
                            idx += 2;
                        } else if sub_mode == 2 && idx + 4 < self.params.len() {
                            // 24-bit TrueColor background
                            let r = self.params[idx + 2] as u8;
                            let g = self.params[idx + 3] as u8;
                            let b = self.params[idx + 4] as u8;
                            grid.current_bg = Color::rgba(r, g, b, 255);
                            idx += 4;
                        }
                    }
                }
                // Default Background
                49 => grid.current_bg = theme.background,
                // Bright Foregrounds
                90..=97 => {
                    let color_idx = (code - 90 + 8) as usize;
                    grid.current_fg = theme.ansi[color_idx];
                }
                // Bright Backgrounds
                100..=107 => {
                    let color_idx = (code - 100 + 8) as usize;
                    grid.current_bg = theme.ansi[color_idx];
                }
                _ => {}
            }
            idx += 1;
        }
    }
}
