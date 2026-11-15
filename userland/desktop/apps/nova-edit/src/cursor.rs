//! userland/desktop/apps/nova-edit/src/cursor.rs
//!
//! Multi-Cursor Editing Engine for NovaOS Code Editor.
//! Coordinates simultaneous carets, selection ranges, and position synchronization.

#[cfg(not(test))]
use alloc::{vec, vec::Vec};

#[cfg(test)]
use std::{vec, vec::Vec};

/// An individual caret with optional selection range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub line: usize,
    pub col: usize,
    pub anchor: Option<(usize, usize)>,
}

impl Cursor {
    pub fn new(line: usize, col: usize) -> Self {
        Self {
            line,
            col,
            anchor: None,
        }
    }
}

/// Manages the primary caret and optional secondary multi-cursors.
#[derive(Clone, Debug)]
pub struct CursorManager {
    pub cursors: Vec<Cursor>,
    pub primary_idx: usize,
}

impl Default for CursorManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CursorManager {
    pub fn new() -> Self {
        Self {
            cursors: vec![Cursor::new(0, 0)],
            primary_idx: 0,
        }
    }

    pub fn primary(&self) -> &Cursor {
        &self.cursors[self.primary_idx]
    }

    pub fn primary_mut(&mut self) -> &mut Cursor {
        &mut self.cursors[self.primary_idx]
    }

    /// Adds a secondary cursor at (line, col).
    pub fn add_cursor(&mut self, line: usize, col: usize) {
        if !self.cursors.iter().any(|c| c.line == line && c.col == col) {
            self.cursors.push(Cursor::new(line, col));
            self.normalize();
        }
    }

    /// Clears all secondary cursors, keeping only the primary caret.
    pub fn clear_secondary(&mut self) {
        let prim = self.cursors[self.primary_idx];
        self.cursors = vec![prim];
        self.primary_idx = 0;
    }

    /// Sorts and merges overlapping cursors.
    pub fn normalize(&mut self) {
        self.cursors.sort_by(|a, b| {
            if a.line == b.line {
                a.col.cmp(&b.col)
            } else {
                a.line.cmp(&b.line)
            }
        });
        self.cursors.dedup_by(|a, b| a.line == b.line && a.col == b.col);
        if self.primary_idx >= self.cursors.len() {
            self.primary_idx = 0;
        }
    }

    /// Moves all cursors left by 1 character.
    pub fn move_left(&mut self) {
        for c in &mut self.cursors {
            if c.col > 0 {
                c.col -= 1;
            }
        }
        self.normalize();
    }

    /// Moves all cursors right by 1 character.
    pub fn move_right(&mut self) {
        for c in &mut self.cursors {
            c.col += 1;
        }
        self.normalize();
    }

    /// Moves all cursors up by 1 line.
    pub fn move_up(&mut self) {
        for c in &mut self.cursors {
            if c.line > 0 {
                c.line -= 1;
            }
        }
        self.normalize();
    }

    /// Moves all cursors down by 1 line.
    pub fn move_down(&mut self, max_lines: usize) {
        for c in &mut self.cursors {
            if c.line + 1 < max_lines {
                c.line += 1;
            }
        }
        self.normalize();
    }
}
