//! userland/desktop/apps/nova-term/src/clipboard.rs
//!
//! Terminal Selection Engine & System Clipboard Integration for NovaTerm.

#[cfg(not(test))]
use alloc::{string::String, string::ToString};

#[cfg(test)]
use std::{string::String, string::ToString};

use crate::grid::TerminalGrid;

/// Selection mode: Linear (standard) or Block (columnar rectangular).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMode {
    Linear,
    Block,
}

/// Active highlighted text selection region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSelection {
    pub start_col: usize,
    pub start_row: usize,
    pub end_col: usize,
    pub end_row: usize,
    pub mode: SelectionMode,
    pub is_active: bool,
}

impl TextSelection {
    pub fn new() -> Self {
        Self {
            start_col: 0,
            start_row: 0,
            end_col: 0,
            end_row: 0,
            mode: SelectionMode::Linear,
            is_active: false,
        }
    }

    /// Begins a selection drag.
    pub fn begin(&mut self, col: usize, row: usize, mode: SelectionMode) {
        self.start_col = col;
        self.start_row = row;
        self.end_col = col;
        self.end_row = row;
        self.mode = mode;
        self.is_active = true;
    }

    /// Updates drag endpoint.
    pub fn update(&mut self, col: usize, row: usize) {
        if self.is_active {
            self.end_col = col;
            self.end_row = row;
        }
    }

    /// Clears selection.
    pub fn clear(&mut self) {
        self.is_active = false;
    }

    /// Tests whether a given character cell is inside the selected area.
    pub fn contains(&self, col: usize, row: usize) -> bool {
        if !self.is_active {
            return false;
        }

        let (r1, r2) = if self.start_row <= self.end_row {
            (self.start_row, self.end_row)
        } else {
            (self.end_row, self.start_row)
        };

        if row < r1 || row > r2 {
            return false;
        }

        match self.mode {
            SelectionMode::Block => {
                let (c1, c2) = if self.start_col <= self.end_col {
                    (self.start_col, self.end_col)
                } else {
                    (self.end_col, self.start_col)
                };
                col >= c1 && col <= c2
            }
            SelectionMode::Linear => {
                if r1 == r2 {
                    let (c1, c2) = if self.start_col <= self.end_col {
                        (self.start_col, self.end_col)
                    } else {
                        (self.end_col, self.start_col)
                    };
                    col >= c1 && col <= c2
                } else if row == r1 {
                    col >= self.start_col
                } else if row == r2 {
                    col <= self.end_col
                } else {
                    true
                }
            }
        }
    }

    /// Extracts selected text from terminal grid.
    pub fn extract_text(&self, grid: &TerminalGrid) -> String {
        if !self.is_active {
            return String::new();
        }

        let (r1, r2) = if self.start_row <= self.end_row {
            (self.start_row, self.end_row)
        } else {
            (self.end_row, self.start_row)
        };

        let mut result = String::new();
        for r in r1..=r2 {
            let mut line = String::new();
            for c in 0..grid.cols {
                if self.contains(c, r) {
                    if let Some(cell) = grid.get_cell(c, r) {
                        line.push(cell.ch);
                    }
                }
            }
            let trimmed = line.trim_end();
            if !trimmed.is_empty() {
                result.push_str(trimmed);
            }
            if r < r2 {
                result.push('\n');
            }
        }
        result
    }
}

/// Clipboard manager holding primary and global clipboard buffers.
pub struct ClipboardManager {
    pub global_clipboard: String,
    pub primary_selection: String,
    pub selection: TextSelection,
}

impl ClipboardManager {
    pub fn new() -> Self {
        Self {
            global_clipboard: String::new(),
            primary_selection: String::new(),
            selection: TextSelection::new(),
        }
    }

    /// Copies string to global clipboard (`Ctrl+Shift+C`).
    pub fn copy(&mut self, text: &str) {
        self.global_clipboard = text.to_string();
        self.primary_selection = text.to_string();
    }

    /// Alias for copy
    pub fn copy_selection(&mut self, text: &str) {
        self.copy(text);
    }

    /// Pastes string from global clipboard (`Ctrl+Shift+V`).
    pub fn paste(&self) -> &str {
        &self.global_clipboard
    }

    /// Gets global clipboard string if non-empty
    pub fn get_clipboard(&self) -> Option<String> {
        if self.global_clipboard.is_empty() {
            None
        } else {
            Some(self.global_clipboard.clone())
        }
    }
}
