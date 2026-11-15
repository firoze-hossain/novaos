//! userland/desktop/apps/nova-edit/src/search.rs
//!
//! Find and Replace Engine with Multi-Match Indexing for NovaOS Code Editor.

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

use crate::buffer::PieceTable;

/// A matched search hit in document coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub start_pos: usize,
    pub end_pos: usize,
}

/// Find and Replace Engine supporting case matching and bulk replacement.
#[derive(Clone, Debug, Default)]
pub struct FindReplaceEngine {
    pub query: String,
    pub replacement: String,
    pub case_sensitive: bool,
    pub is_active: bool,
    pub matches: Vec<SearchMatch>,
    pub current_idx: usize,
}

impl FindReplaceEngine {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            replacement: String::new(),
            case_sensitive: false,
            is_active: false,
            matches: Vec::new(),
            current_idx: 0,
        }
    }

    /// Finds all occurrences of `query` in `text`.
    pub fn find_all(text: &str, query: &str, case_sensitive: bool) -> Vec<SearchMatch> {
        if query.is_empty() || text.is_empty() {
            return Vec::new();
        }

        let mut matches = Vec::new();
        let q_len = query.len();

        if case_sensitive {
            let mut start = 0;
            while let Some(pos) = text[start..].find(query) {
                let abs_pos = start + pos;
                matches.push(SearchMatch {
                    start_pos: abs_pos,
                    end_pos: abs_pos + q_len,
                });
                start = abs_pos + q_len.max(1);
                if start >= text.len() {
                    break;
                }
            }
        } else {
            let text_lower = text.to_lowercase();
            let query_lower = query.to_lowercase();
            let mut start = 0;
            while let Some(pos) = text_lower[start..].find(&query_lower) {
                let abs_pos = start + pos;
                matches.push(SearchMatch {
                    start_pos: abs_pos,
                    end_pos: abs_pos + q_len,
                });
                start = abs_pos + q_len.max(1);
                if start >= text_lower.len() {
                    break;
                }
            }
        }

        matches
    }

    /// Updates active search query and recomputes match list.
    pub fn set_query(&mut self, text: &str, query: &str, case_sensitive: bool) {
        self.query = query.to_string();
        self.case_sensitive = case_sensitive;
        self.matches = Self::find_all(text, query, case_sensitive);
        self.current_idx = 0;
        self.is_active = !query.is_empty();
    }

    /// Advances to the next search match.
    pub fn next_match(&mut self) -> Option<SearchMatch> {
        if self.matches.is_empty() {
            None
        } else {
            self.current_idx = (self.current_idx + 1) % self.matches.len();
            Some(self.matches[self.current_idx])
        }
    }

    /// Replaces all occurrences in the piece table in reverse order to preserve offsets.
    pub fn replace_all(
        table: &mut PieceTable,
        query: &str,
        replacement: &str,
        case_sensitive: bool,
    ) -> usize {
        let full_text = table.get_text();
        let matches = Self::find_all(&full_text, query, case_sensitive);
        let count = matches.len();

        for m in matches.iter().rev() {
            table.delete(m.start_pos, m.end_pos - m.start_pos);
            table.insert(m.start_pos, replacement);
        }

        count
    }
}
