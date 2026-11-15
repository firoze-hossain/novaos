//! userland/desktop/apps/nova-term/src/grid.rs
//!
//! Terminal Cell Grid, Scrollback Buffer, and TrueColor Styling for NovaTerm.

use crate::theme::ColorScheme;
use nova_ui::prelude::Color;

#[cfg(not(test))]
use alloc::{vec, vec::Vec};

#[cfg(test)]
use std::{vec, vec::Vec};

/// Bitflags representing character cell formatting attributes.
pub const FLAG_BOLD: u8 = 1 << 0;
pub const FLAG_DIM: u8 = 1 << 1;
pub const FLAG_ITALIC: u8 = 1 << 2;
pub const FLAG_UNDERLINE: u8 = 1 << 3;
pub const FLAG_BLINK: u8 = 1 << 4;
pub const FLAG_INVERSE: u8 = 1 << 5;
pub const FLAG_STRIKETHROUGH: u8 = 1 << 6;

/// A single character cell on the terminal screen with 24-bit TrueColor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: u8,
}

impl Cell {
    pub const fn empty(fg: Color, bg: Color) -> Self {
        Self {
            ch: ' ',
            fg,
            bg,
            flags: 0,
        }
    }

    pub fn is_bold(&self) -> bool {
        (self.flags & FLAG_BOLD) != 0
    }

    pub fn is_italic(&self) -> bool {
        (self.flags & FLAG_ITALIC) != 0
    }

    pub fn is_underline(&self) -> bool {
        (self.flags & FLAG_UNDERLINE) != 0
    }

    pub fn is_inverse(&self) -> bool {
        (self.flags & FLAG_INVERSE) != 0
    }
}

/// The 2D terminal grid managing cursor, primary/alternate screen buffers, and scrollback.
pub struct TerminalGrid {
    pub cols: usize,
    pub rows: usize,
    pub primary_cells: Vec<Cell>,
    pub alt_cells: Vec<Cell>,
    pub is_alt_screen: bool,
    pub scrollback: Vec<Vec<Cell>>,
    pub max_scrollback_lines: usize,
    pub scroll_offset: usize,
    pub cursor_x: usize,
    pub cursor_y: usize,
    pub saved_cursor_x: usize,
    pub saved_cursor_y: usize,
    pub cursor_visible: bool,
    pub current_fg: Color,
    pub current_bg: Color,
    pub current_flags: u8,
    pub scroll_top: usize,
    pub scroll_bottom: usize,
}

impl TerminalGrid {
    pub fn new(cols: usize, rows: usize, theme: &ColorScheme) -> Self {
        let count = cols * rows;
        let default_cell = Cell::empty(theme.foreground, theme.background);

        Self {
            cols,
            rows,
            primary_cells: vec![default_cell; count],
            alt_cells: vec![default_cell; count],
            is_alt_screen: false,
            scrollback: Vec::new(),
            max_scrollback_lines: 5000,
            scroll_offset: 0,
            cursor_x: 0,
            cursor_y: 0,
            saved_cursor_x: 0,
            saved_cursor_y: 0,
            cursor_visible: true,
            current_fg: theme.foreground,
            current_bg: theme.background,
            current_flags: 0,
            scroll_top: 0,
            scroll_bottom: rows.saturating_sub(1),
        }
    }

    fn active_cells(&self) -> &[Cell] {
        if self.is_alt_screen {
            &self.alt_cells
        } else {
            &self.primary_cells
        }
    }

    fn active_cells_mut(&mut self) -> &mut [Cell] {
        if self.is_alt_screen {
            &mut self.alt_cells
        } else {
            &mut self.primary_cells
        }
    }

    /// Retrieves cell at column x and row y.
    pub fn get_cell(&self, x: usize, y: usize) -> Option<Cell> {
        if x < self.cols && y < self.rows {
            Some(self.active_cells()[y * self.cols + x])
        } else {
            None
        }
    }

    /// Sets cell at column x and row y.
    pub fn set_cell(&mut self, x: usize, y: usize, cell: Cell) {
        if x < self.cols && y < self.rows {
            let idx = y * self.cols + x;
            self.active_cells_mut()[idx] = cell;
        }
    }

    /// Writes a printable character at current cursor position.
    pub fn put_char(&mut self, ch: char) {
        if self.cursor_x >= self.cols {
            self.carriage_return();
            self.line_feed();
        }

        let cell = Cell {
            ch,
            fg: self.current_fg,
            bg: self.current_bg,
            flags: self.current_flags,
        };

        let x = self.cursor_x;
        let y = self.cursor_y;
        self.set_cell(x, y, cell);
        self.cursor_x += 1;
    }

    /// Carriage return `\r`.
    pub fn carriage_return(&mut self) {
        self.cursor_x = 0;
    }

    /// Line feed `\n` with scrolling within margins.
    pub fn line_feed(&mut self) {
        if self.cursor_y < self.scroll_bottom {
            self.cursor_y += 1;
        } else {
            self.scroll_up(1);
        }
    }

    /// Backspace `\b`.
    pub fn backspace(&mut self) {
        if self.cursor_x > 0 {
            self.cursor_x -= 1;
        }
    }

    /// Horizontal tab `\t` (8-column stops).
    pub fn tab(&mut self) {
        let next_tab = (self.cursor_x + 8) & !7;
        self.cursor_x = next_tab.min(self.cols.saturating_sub(1));
    }

    /// Scrolls screen content up by `lines` count within scroll region.
    pub fn scroll_up(&mut self, lines: usize) {
        let cols = self.cols;
        let empty_cell = Cell::empty(self.current_fg, self.current_bg);

        for _ in 0..lines {
            // Save top row into scrollback if on primary buffer and full-height scroll
            if !self.is_alt_screen && self.scroll_top == 0 {
                let top_row: Vec<Cell> = (0..cols).map(|x| self.primary_cells[x]).collect();
                if self.scrollback.len() >= self.max_scrollback_lines {
                    self.scrollback.remove(0);
                }
                self.scrollback.push(top_row);
            }

            // Shift rows up within scroll region
            for y in self.scroll_top..self.scroll_bottom {
                for x in 0..cols {
                    let next_val = self.active_cells()[(y + 1) * cols + x];
                    let idx = y * cols + x;
                    self.active_cells_mut()[idx] = next_val;
                }
            }

            // Clear bottom row of region
            let bottom_y = self.scroll_bottom;
            for x in 0..cols {
                let idx = bottom_y * cols + x;
                self.active_cells_mut()[idx] = empty_cell;
            }
        }
    }

    /// Scrolls screen content down by `lines` count.
    pub fn scroll_down(&mut self, lines: usize) {
        let cols = self.cols;
        let empty_cell = Cell::empty(self.current_fg, self.current_bg);

        for _ in 0..lines {
            for y in (self.scroll_top + 1..=self.scroll_bottom).rev() {
                for x in 0..cols {
                    let prev_val = self.active_cells()[(y - 1) * cols + x];
                    let idx = y * cols + x;
                    self.active_cells_mut()[idx] = prev_val;
                }
            }

            for x in 0..cols {
                let idx = self.scroll_top * cols + x;
                self.active_cells_mut()[idx] = empty_cell;
            }
        }
    }

    /// Clears display according to mode (0: below, 1: above, 2: all).
    pub fn clear_screen(&mut self, mode: u32) {
        let empty_cell = Cell::empty(self.current_fg, self.current_bg);
        let cur_idx = self.cursor_y * self.cols + self.cursor_x;

        match mode {
            0 => {
                // Clear from cursor to end of screen
                for idx in cur_idx..self.active_cells().len() {
                    self.active_cells_mut()[idx] = empty_cell;
                }
            }
            1 => {
                // Clear from beginning to cursor
                for idx in 0..=cur_idx.min(self.active_cells().len() - 1) {
                    self.active_cells_mut()[idx] = empty_cell;
                }
            }
            2 | 3 => {
                // Clear entire screen
                for cell in self.active_cells_mut().iter_mut() {
                    *cell = empty_cell;
                }
                self.cursor_x = 0;
                self.cursor_y = 0;
            }
            _ => {}
        }
    }

    /// Clears line according to mode (0: to right, 1: to left, 2: all).
    pub fn clear_line(&mut self, mode: u32) {
        let empty_cell = Cell::empty(self.current_fg, self.current_bg);
        let y = self.cursor_y;
        let cols = self.cols;

        match mode {
            0 => {
                for x in self.cursor_x..cols {
                    self.set_cell(x, y, empty_cell);
                }
            }
            1 => {
                for x in 0..=self.cursor_x.min(cols - 1) {
                    self.set_cell(x, y, empty_cell);
                }
            }
            2 => {
                for x in 0..cols {
                    self.set_cell(x, y, empty_cell);
                }
            }
            _ => {}
        }
    }

    /// Sets cursor position (0-indexed).
    pub fn set_cursor(&mut self, x: usize, y: usize) {
        self.cursor_x = x.min(self.cols.saturating_sub(1));
        self.cursor_y = y.min(self.rows.saturating_sub(1));
    }

    /// Saves cursor position.
    pub fn save_cursor(&mut self) {
        self.saved_cursor_x = self.cursor_x;
        self.saved_cursor_y = self.cursor_y;
    }

    /// Restores cursor position.
    pub fn restore_cursor(&mut self) {
        self.cursor_x = self.saved_cursor_x.min(self.cols.saturating_sub(1));
        self.cursor_y = self.saved_cursor_y.min(self.rows.saturating_sub(1));
    }

    /// Switches alternate screen buffer (e.g. for vim, htop, less).
    pub fn set_alt_screen(&mut self, enable: bool) {
        self.is_alt_screen = enable;
        if enable {
            self.cursor_x = 0;
            self.cursor_y = 0;
        }
    }

    /// Dynamic terminal resizing with grid reflow.
    pub fn resize(&mut self, new_cols: usize, new_rows: usize, theme: &ColorScheme) {
        if new_cols == 0 || new_rows == 0 {
            return;
        }

        let new_count = new_cols * new_rows;
        let empty_cell = Cell::empty(theme.foreground, theme.background);

        let mut new_primary = vec![empty_cell; new_count];
        let mut new_alt = vec![empty_cell; new_count];

        let copy_cols = self.cols.min(new_cols);
        let copy_rows = self.rows.min(new_rows);

        for y in 0..copy_rows {
            for x in 0..copy_cols {
                new_primary[y * new_cols + x] = self.primary_cells[y * self.cols + x];
                new_alt[y * new_cols + x] = self.alt_cells[y * self.cols + x];
            }
        }

        self.cols = new_cols;
        self.rows = new_rows;
        self.primary_cells = new_primary;
        self.alt_cells = new_alt;
        self.scroll_top = 0;
        self.scroll_bottom = new_rows.saturating_sub(1);
        self.cursor_x = self.cursor_x.min(new_cols.saturating_sub(1));
        self.cursor_y = self.cursor_y.min(new_rows.saturating_sub(1));
    }
}
