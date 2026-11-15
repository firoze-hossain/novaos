//! userland/desktop/apps/nova-edit/src/folding.rs
//!
//! Code Folding Engine for NovaOS Code Editor.
//! Identifies block scopes, manages folded/unfolded state, and maps visible line ranges.

#[cfg(not(test))]
use alloc::{string::String, vec::Vec};

#[cfg(test)]
use std::{string::String, vec::Vec};

/// A collapsible block of code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldRange {
    pub start_line: usize,
    pub end_line: usize,
    pub is_folded: bool,
}

/// Manages code folding regions and visibility maps.
#[derive(Clone, Debug, Default)]
pub struct CodeFoldingManager {
    pub ranges: Vec<FoldRange>,
}

impl CodeFoldingManager {
    pub fn new() -> Self {
        Self { ranges: Vec::new() }
    }

    /// Computes foldable scopes from braces `{` and `}` or block starts.
    pub fn calculate_folds(&mut self, lines: &[String]) {
        self.ranges.clear();
        let mut stack = Vec::new();

        for (idx, line) in lines.iter().enumerate() {
            let bytes = line.as_bytes();
            for &b in bytes {
                if b == b'{' {
                    stack.push(idx);
                } else if b == b'}' {
                    if let Some(start_line) = stack.pop() {
                        if idx > start_line {
                            self.ranges.push(FoldRange {
                                start_line,
                                end_line: idx,
                                is_folded: false,
                            });
                        }
                    }
                }
            }
        }
    }

    /// Toggles fold state for the region starting on `line`.
    pub fn toggle_fold(&mut self, line: usize) -> bool {
        if let Some(range) = self.ranges.iter_mut().find(|r| r.start_line == line) {
            range.is_folded = !range.is_folded;
            true
        } else {
            false
        }
    }

    /// Checks if a line is hidden due to an active folded scope above it.
    pub fn is_line_visible(&self, line: usize) -> bool {
        for range in &self.ranges {
            if range.is_folded && line > range.start_line && line <= range.end_line {
                return false;
            }
        }
        true
    }

    /// Returns the gutter fold indicator for a line if it is the start of a foldable range.
    pub fn fold_marker(&self, line: usize) -> Option<&'static str> {
        self.ranges.iter().find(|r| r.start_line == line).map(|r| {
            if r.is_folded {
                "[+]"
            } else {
                "[-]"
            }
        })
    }
}
