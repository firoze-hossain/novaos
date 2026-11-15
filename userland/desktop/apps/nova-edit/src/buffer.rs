//! userland/desktop/apps/nova-edit/src/buffer.rs
//!
//! Piece-Table Text Buffer Engine for NovaOS Code Editor.
//! Delivers O(1) edits, multi-megabyte instant file opening, line indexing,
//! and full Undo/Redo history tracking.

#[cfg(not(test))]
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

#[cfg(test)]
use std::{
    string::{String, ToString},
    vec::Vec,
};

/// Buffer source type for a piece table entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferKind {
    Original,
    Add,
}

/// A slice descriptor referencing a portion of the original or append buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub kind: BufferKind,
    pub start: usize,
    pub length: usize,
}

/// Piece-table text buffer with undo/redo history.
#[derive(Clone, Debug)]
pub struct PieceTable {
    pub original: String,
    pub add: String,
    pub pieces: Vec<Piece>,
    pub undo_stack: Vec<Vec<Piece>>,
    pub redo_stack: Vec<Vec<Piece>>,
    pub is_dirty: bool,
    pub total_length: usize,
    cached_lines: Option<Vec<String>>,
}

impl Default for PieceTable {
    fn default() -> Self {
        Self::new("")
    }
}

impl PieceTable {
    pub fn new(initial_text: &str) -> Self {
        let original = initial_text.to_string();
        let len = original.len();
        let mut pieces = Vec::new();
        if len > 0 {
            pieces.push(Piece {
                kind: BufferKind::Original,
                start: 0,
                length: len,
            });
        }

        Self {
            original,
            add: String::new(),
            pieces,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            is_dirty: false,
            total_length: len,
            cached_lines: None,
        }
    }

    /// Total byte length of the reconstructed document.
    pub fn len(&self) -> usize {
        self.total_length
    }

    pub fn is_empty(&self) -> bool {
        self.total_length == 0
    }

    /// Reconstructs the complete document text from pieces.
    pub fn get_text(&self) -> String {
        let mut out = String::with_capacity(self.total_length);
        for p in &self.pieces {
            let slice = match p.kind {
                BufferKind::Original => &self.original[p.start..p.start + p.length],
                BufferKind::Add => &self.add[p.start..p.start + p.length],
            };
            out.push_str(slice);
        }
        out
    }

    /// Saves the current piece sequence onto the undo stack before modifying.
    fn push_undo(&mut self) {
        self.undo_stack.push(self.pieces.clone());
        if self.undo_stack.len() > 100 {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
        self.is_dirty = true;
        self.cached_lines = None;
    }

    /// Inserts text at the given character/byte offset.
    pub fn insert(&mut self, offset: usize, text: &str) {
        if text.is_empty() {
            return;
        }

        self.push_undo();
        let ins_len = text.len();
        let add_start = self.add.len();
        self.add.push_str(text);

        let new_piece = Piece {
            kind: BufferKind::Add,
            start: add_start,
            length: ins_len,
        };

        if self.pieces.is_empty() {
            self.pieces.push(new_piece);
            self.total_length = ins_len;
            return;
        }

        let mut cur_offset = 0;
        let mut split_idx = self.pieces.len();
        let mut split_offset_in_piece = 0;

        for (i, p) in self.pieces.iter().enumerate() {
            if offset <= cur_offset + p.length {
                split_idx = i;
                split_offset_in_piece = offset - cur_offset;
                break;
            }
            cur_offset += p.length;
        }

        if split_idx >= self.pieces.len() {
            // Append at the end
            self.pieces.push(new_piece);
        } else if split_offset_in_piece == 0 {
            // Insert before piece
            self.pieces.insert(split_idx, new_piece);
        } else if split_offset_in_piece == self.pieces[split_idx].length {
            // Insert after piece
            self.pieces.insert(split_idx + 1, new_piece);
        } else {
            // Split piece into left and right
            let target_piece = self.pieces[split_idx].clone();
            let left_piece = Piece {
                kind: target_piece.kind,
                start: target_piece.start,
                length: split_offset_in_piece,
            };
            let right_piece = Piece {
                kind: target_piece.kind,
                start: target_piece.start + split_offset_in_piece,
                length: target_piece.length - split_offset_in_piece,
            };

            self.pieces[split_idx] = left_piece;
            self.pieces.insert(split_idx + 1, new_piece);
            self.pieces.insert(split_idx + 2, right_piece);
        }

        self.total_length += ins_len;
    }

    /// Deletes text starting at offset for length bytes.
    pub fn delete(&mut self, offset: usize, length: usize) {
        if length == 0 || offset >= self.total_length {
            return;
        }

        let del_len = length.min(self.total_length - offset);
        self.push_undo();

        let del_end = offset + del_len;
        let mut new_pieces = Vec::new();
        let mut cur_offset = 0;

        for p in &self.pieces {
            let piece_start = cur_offset;
            let piece_end = cur_offset + p.length;

            if piece_end <= offset || piece_start >= del_end {
                // Completely outside delete region -> keep piece
                new_pieces.push(p.clone());
            } else {
                // Intersects with delete region
                if piece_start < offset {
                    // Keep left part
                    new_pieces.push(Piece {
                        kind: p.kind,
                        start: p.start,
                        length: offset - piece_start,
                    });
                }
                if piece_end > del_end {
                    // Keep right part
                    let consumed = del_end - piece_start;
                    new_pieces.push(Piece {
                        kind: p.kind,
                        start: p.start + consumed,
                        length: p.length - consumed,
                    });
                }
            }

            cur_offset = piece_end;
        }

        self.pieces = new_pieces;
        self.total_length -= del_len;
    }

    /// Undoes the last edit operation.
    pub fn undo(&mut self) -> bool {
        if let Some(prev) = self.undo_stack.pop() {
            self.redo_stack.push(self.pieces.clone());
            self.pieces = prev;
            self.total_length = self.pieces.iter().map(|p| p.length).sum();
            self.cached_lines = None;
            true
        } else {
            false
        }
    }

    /// Redoes the previously undone edit.
    pub fn redo(&mut self) -> bool {
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push(self.pieces.clone());
            self.pieces = next;
            self.total_length = self.pieces.iter().map(|p| p.length).sum();
            self.cached_lines = None;
            true
        } else {
            false
        }
    }

    /// Marks buffer as clean and saved.
    pub fn mark_saved(&mut self) {
        self.is_dirty = false;
    }

    /// Rebuilds cached lines if necessary and returns slice of lines.
    pub fn lines(&mut self) -> &[String] {
        if self.cached_lines.is_none() {
            let full_text = self.get_text();
            let mut lines = Vec::new();
            if full_text.is_empty() {
                lines.push(String::new());
            } else {
                for line in full_text.split('\n') {
                    // Strip optional \r
                    let clean = line.strip_suffix('\r').unwrap_or(line);
                    lines.push(clean.to_string());
                }
            }
            self.cached_lines = Some(lines);
        }
        self.cached_lines.as_ref().unwrap()
    }

    /// Returns total number of lines.
    pub fn line_count(&mut self) -> usize {
        self.lines().len()
    }

    /// Returns the text of a specific line (0-indexed).
    pub fn get_line(&mut self, line_idx: usize) -> Option<String> {
        self.lines().get(line_idx).cloned()
    }

    /// Converts byte/character offset to (line, col).
    pub fn pos_to_line_col(&mut self, offset: usize) -> (usize, usize) {
        let lines = self.lines();
        let mut cur = 0;
        for (line_idx, l) in lines.iter().enumerate() {
            let line_len = l.len() + 1; // +1 for '\n'
            if offset < cur + line_len {
                let col = offset - cur;
                return (line_idx, col.min(l.len()));
            }
            cur += line_len;
        }
        (lines.len().saturating_sub(1), lines.last().map(|l| l.len()).unwrap_or(0))
    }

    /// Converts (line, col) to byte/character offset.
    pub fn line_col_to_pos(&mut self, line: usize, col: usize) -> usize {
        let lines = self.lines();
        let mut cur = 0;
        for (idx, l) in lines.iter().enumerate() {
            if idx == line {
                return cur + col.min(l.len());
            }
            cur += l.len() + 1; // +1 for '\n'
        }
        cur
    }
}
