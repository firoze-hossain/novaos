//! userland/desktop/apps/nova-term/src/url.rs
//!
//! Smart URL Detection, Underline on Hover, and Click-to-Open for NovaTerm.

#[cfg(not(test))]
use alloc::{string::String, string::ToString, vec::Vec};

#[cfg(test)]
use std::{string::String, string::ToString, vec::Vec};

use crate::grid::TerminalGrid;

/// A detected URL occurrence on a specific line of the terminal grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedUrl {
    pub url: String,
    pub row: usize,
    pub start_col: usize,
    pub end_col: usize,
}

/// URL detection scanner and click-to-open engine.
pub struct UrlScanner {
    pub detected_urls: Vec<DetectedUrl>,
    pub hovered_url: Option<String>,
    pub last_opened_url: Option<String>,
}

impl UrlScanner {
    pub fn new() -> Self {
        Self {
            detected_urls: Vec::new(),
            hovered_url: None,
            last_opened_url: None,
        }
    }

    /// Scans a text line for URLs (http://, https://, file://, ftp://).
    pub fn scan_line(&mut self, row: usize, line: &str) {
        let prefixes = ["http://", "https://", "file://", "ftp://"];
        let mut search_idx = 0;

        while search_idx < line.len() {
            let mut found_prefix = None;
            let mut earliest_pos = line.len();

            for &p in &prefixes {
                if let Some(pos) = line[search_idx..].find(p) {
                    let actual_pos = search_idx + pos;
                    if actual_pos < earliest_pos {
                        earliest_pos = actual_pos;
                        found_prefix = Some(p);
                    }
                }
            }

            if let Some(_p) = found_prefix {
                let start = earliest_pos;
                // Find termination character (whitespace, quote, angle bracket, parenthesis)
                let end = line[start..]
                    .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '>' || c == ')' || c == ']')
                    .map(|offset| start + offset)
                    .unwrap_or(line.len());

                let raw_url = &line[start..end];
                // Trim trailing punctuation if any
                let clean_url = raw_url.trim_end_matches('.').trim_end_matches(',');

                self.detected_urls.push(DetectedUrl {
                    url: clean_url.to_string(),
                    row,
                    start_col: start,
                    end_col: start + clean_url.len(),
                });

                search_idx = end;
            } else {
                break;
            }
        }
    }

    /// Hit-tests a character cell coordinate (col, row) to see if a URL is located there.
    pub fn hit_test(&self, col: usize, row: usize) -> Option<&DetectedUrl> {
        self.detected_urls.iter().find(|u| u.row == row && col >= u.start_col && col < u.end_col)
    }

    /// Updates hover state.
    pub fn update_hover(&mut self, col: usize, row: usize) -> bool {
        let hit = self.hit_test(col, row);
        let new_hover = hit.map(|u| u.url.clone());
        let changed = self.hovered_url != new_hover;
        self.hovered_url = new_hover;
        changed
    }

    /// Clicks on a URL coordinate, opening it in the system browser.
    pub fn click_at(&mut self, col: usize, row: usize) -> Option<String> {
        if let Some(hit) = self.hit_test(col, row) {
            let url = hit.url.clone();
            self.last_opened_url = Some(url.clone());
            Some(url)
        } else {
            None
        }
    }

    /// Clears scanned cache before re-scan.
    pub fn clear(&mut self) {
        self.detected_urls.clear();
        self.hovered_url = None;
    }

    /// Scans all rows in the terminal grid for URLs and caches detected instances.
    pub fn scan_grid(&mut self, grid: &TerminalGrid) -> &[DetectedUrl] {
        self.clear();
        for r in 0..grid.rows {
            let mut line = String::new();
            for c in 0..grid.cols {
                if let Some(cell) = grid.get_cell(c, r) {
                    line.push(cell.ch);
                }
            }
            self.scan_line(r, &line);
        }
        &self.detected_urls
    }

    /// Checks if a cell contains a detected URL.
    pub fn is_url_cell(&self, col: usize, row: usize) -> bool {
        self.hit_test(col, row).is_some()
    }

    /// Checks if a cell is part of the currently hovered URL.
    pub fn is_hovered(&self, col: usize, row: usize) -> bool {
        if let Some(hover) = &self.hovered_url {
            if let Some(detected) = self.hit_test(col, row) {
                return detected.url == *hover;
            }
        }
        false
    }

    /// Sets hover state at cell coordinates and returns matched URL.
    pub fn set_hover(&mut self, col: usize, row: usize) -> Option<&DetectedUrl> {
        let hit = self.hit_test(col, row);
        self.hovered_url = hit.map(|u| u.url.clone());
        self.hit_test(col, row)
    }

    /// Gets URL at specified cell coordinates.
    pub fn url_at(&self, col: usize, row: usize) -> Option<&DetectedUrl> {
        self.hit_test(col, row)
    }
}
