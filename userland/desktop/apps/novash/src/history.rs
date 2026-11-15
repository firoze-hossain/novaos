//! userland/desktop/apps/novash/src/history.rs
//!
//! Command History Management and Fish-Style Autosuggestions for NovaShell.
//! Tracks executed commands, handles history file persistence, and generates
//! real-time ghost text suggestions ahead of the cursor.

#[cfg(not(test))]
use alloc::{string::String, string::ToString, vec::Vec};
#[cfg(test)]
use std::{string::String, string::ToString, vec::Vec};

/// An autosuggestion match representing ghost text ahead of the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Autosuggestion {
    pub full_match: String,
    pub suffix: String,
}

/// Command history manager with autosuggestion support.
pub struct CommandHistory {
    pub entries: Vec<String>,
    pub max_entries: usize,
    pub browse_index: Option<usize>,
    pub saved_current_line: String,
}

impl CommandHistory {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            max_entries: 5000,
            browse_index: None,
            saved_current_line: String::new(),
        }
    }

    /// Add a command to history (skipping empty or duplicate consecutive commands)
    pub fn add(&mut self, line: &str) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }

        if let Some(last) = self.entries.last() {
            if last == trimmed {
                return;
            }
        }

        if self.entries.len() >= self.max_entries {
            self.entries.remove(0);
        }

        self.entries.push(String::from(trimmed));
        self.browse_index = None;
    }

    /// Reset history navigation pointer
    pub fn reset_browse(&mut self) {
        self.browse_index = None;
        self.saved_current_line.clear();
    }

    /// Navigate backward in history (Up Arrow)
    pub fn browse_prev(&mut self, current_line: &str) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }

        match self.browse_index {
            None => {
                self.saved_current_line = current_line.to_string();
                let last_idx = self.entries.len() - 1;
                self.browse_index = Some(last_idx);
                Some(&self.entries[last_idx])
            }
            Some(idx) => {
                if idx > 0 {
                    let prev_idx = idx - 1;
                    self.browse_index = Some(prev_idx);
                    Some(&self.entries[prev_idx])
                } else {
                    Some(&self.entries[0])
                }
            }
        }
    }

    /// Navigate forward in history (Down Arrow)
    pub fn browse_next(&mut self) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }

        match self.browse_index {
            None => None,
            Some(idx) => {
                if idx + 1 < self.entries.len() {
                    let next_idx = idx + 1;
                    self.browse_index = Some(next_idx);
                    Some(&self.entries[next_idx])
                } else {
                    self.browse_index = None;
                    Some(&self.saved_current_line)
                }
            }
        }
    }

    /// Generates Fish-style autosuggestion for the given prefix
    pub fn suggest(&self, prefix: &str) -> Option<Autosuggestion> {
        let p_trimmed = prefix.trim_start();
        if p_trimmed.is_empty() {
            return None;
        }

        // Search backward from most recent history entries
        for entry in self.entries.iter().rev() {
            if entry.starts_with(prefix) && entry.len() > prefix.len() {
                let suffix = entry[prefix.len()..].to_string();
                return Some(Autosuggestion {
                    full_match: entry.clone(),
                    suffix,
                });
            }
        }

        None
    }

    /// Computes next word from autosuggestion suffix (for Alt+Right arrow partial acceptance)
    pub fn next_suggestion_word(suffix: &str) -> &str {
        let mut in_leading_space = true;

        for (idx, ch) in suffix.char_indices() {
            if in_leading_space {
                if !ch.is_whitespace() {
                    in_leading_space = false;
                }
            } else if ch.is_whitespace() || ch == '/' || ch == '-' {
                let end_idx = idx + ch.len_utf8();
                return &suffix[..end_idx];
            }
        }

        suffix
    }
}
