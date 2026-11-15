//! userland/desktop/apps/nova-edit/src/tabs.rs
//!
//! Multi-File Tab Management and Auto-Save Subsystem for NovaOS Code Editor.

#[cfg(not(test))]
use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[cfg(test)]
use std::{
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::buffer::PieceTable;
use crate::cursor::CursorManager;
use crate::folding::CodeFoldingManager;
use crate::syntax::Language;

/// An open file tab containing its own piece-table buffer, cursors, and folding map.
#[derive(Clone, Debug)]
pub struct EditorDocument {
    pub path: String,
    pub title: String,
    pub language: Language,
    pub buffer: PieceTable,
    pub cursors: CursorManager,
    pub folding: CodeFoldingManager,
    pub scroll_line: usize,
    pub scroll_col: usize,
}

impl EditorDocument {
    pub fn new(path: &str, content: &str) -> Self {
        let title = path.split('/').last().unwrap_or(path).to_string();
        let language = Language::detect_from_path(path);
        let mut buffer = PieceTable::new(content);

        let mut folding = CodeFoldingManager::new();
        folding.calculate_folds(buffer.lines());

        Self {
            path: path.to_string(),
            title,
            language,
            buffer,
            cursors: CursorManager::new(),
            folding,
            scroll_line: 0,
            scroll_col: 0,
        }
    }
}

/// Tab manager coordinating open editor documents.
#[derive(Clone, Debug)]
pub struct TabManager {
    pub tabs: Vec<EditorDocument>,
    pub active_idx: usize,
}

impl Default for TabManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TabManager {
    pub fn new() -> Self {
        let default_doc = EditorDocument::new(
            "src/main.rs",
            "//! NovaOS Kernel Entry Point\nfn main() {\n    println!(\"Hello, NovaOS 2.0!\");\n}\n",
        );
        Self {
            tabs: vec![default_doc],
            active_idx: 0,
        }
    }

    pub fn active(&self) -> &EditorDocument {
        &self.tabs[self.active_idx]
    }

    pub fn active_mut(&mut self) -> &mut EditorDocument {
        &mut self.tabs[self.active_idx]
    }

    /// Opens a file in a new tab, or switches to it if already open.
    pub fn open_file(&mut self, path: &str, content: &str) {
        if let Some(idx) = self.tabs.iter().position(|t| t.path == path) {
            self.active_idx = idx;
        } else {
            let doc = EditorDocument::new(path, content);
            self.tabs.push(doc);
            self.active_idx = self.tabs.len() - 1;
        }
    }

    /// Closes the tab at `idx`.
    pub fn close_tab(&mut self, idx: usize) -> bool {
        if self.tabs.len() <= 1 {
            // Keep at least one tab open
            return false;
        }
        if idx < self.tabs.len() {
            self.tabs.remove(idx);
            if self.active_idx >= self.tabs.len() {
                self.active_idx = self.tabs.len() - 1;
            }
            true
        } else {
            false
        }
    }

    /// Switches the active document tab.
    pub fn switch_tab(&mut self, idx: usize) -> bool {
        if idx < self.tabs.len() {
            self.active_idx = idx;
            true
        } else {
            false
        }
    }

    /// Auto-saves all modified buffers and marks them clean.
    pub fn auto_save_all(&mut self) -> usize {
        let mut count = 0;
        for tab in &mut self.tabs {
            if tab.buffer.is_dirty {
                tab.buffer.mark_saved();
                count += 1;
            }
        }
        count
    }
}
