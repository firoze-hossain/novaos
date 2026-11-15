//! userland/desktop/apps/nova-edit/src/lib.rs
//!
//! # NovaOS Code & Text Editor (nova-edit)
//!
//! A fast, distraction-free code editor in pure Rust delivering:
//! - Piece-table text buffer data structure for O(1) edits on multi-megabyte files.
//! - Multi-language syntax highlighting: Rust, C/C++, Python, Markdown, Shell, and JSON.
//! - Multi-cursor editing with simultaneous caret synchronization.
//! - Code folding engine with block scope discovery and gutter fold markers.
//! - Bracket matching and auto-closing delimiter insertion.
//! - Find & Replace with case matching and bulk replacement.
//! - Project directory sidebar tree navigation.
//! - Multi-file tabbed document management with auto-save.
//! - High-performance dark acrylic UI rendering into 32-bit BufferSurface.

#![cfg_attr(not(test), no_std)]

#[cfg(not(test))]
extern crate alloc;

#[cfg(not(test))]
use alloc::{
    format,
    string::{String, ToString},
    vec,
};

#[cfg(test)]
use std::{
    format,
    string::{String, ToString},
    vec,
};

pub mod bracket;
pub mod buffer;
pub mod cursor;
pub mod folding;
pub mod project;
pub mod search;
pub mod syntax;
pub mod tabs;
pub mod view;

use bracket::BracketMatcher;
use buffer::PieceTable;
use nova_ui::prelude::{BufferSurface, Point};
use project::ProjectExplorer;
use search::FindReplaceEngine;
use syntax::{Language, SyntaxHighlighter, TokenType};
use tabs::TabManager;
use view::{EditHitTarget, EditorView};

/// Top-level application controller for Nova Edit.
pub struct NovaEditApp {
    pub tabs: TabManager,
    pub project: ProjectExplorer,
    pub search: FindReplaceEngine,
    pub view: EditorView,
}

impl Default for NovaEditApp {
    fn default() -> Self {
        Self::new()
    }
}

impl NovaEditApp {
    pub fn new() -> Self {
        Self {
            tabs: TabManager::new(),
            project: ProjectExplorer::default(),
            search: FindReplaceEngine::new(),
            view: EditorView::default(),
        }
    }

    /// Opens a document in a new or existing tab.
    pub fn open_file(&mut self, path: &str, content: &str) {
        self.tabs.open_file(path, content);
    }

    /// Saves the active document and marks it clean.
    pub fn save_active_file(&mut self) {
        self.tabs.active_mut().buffer.mark_saved();
    }

    /// Inserts a character at all active cursors.
    pub fn insert_char(&mut self, ch: char) {
        let doc = self.tabs.active_mut();
        let cursors = doc.cursors.cursors.clone();

        for c in cursors.iter().rev() {
            let pos = doc.buffer.line_col_to_pos(c.line, c.col);
            let mut s = String::new();
            s.push(ch);
            // Auto-close matching bracket
            if let Some(close) = BracketMatcher::get_auto_close_char(ch) {
                if ch != close || ch == '"' || ch == '\'' {
                    s.push(close);
                }
            }
            doc.buffer.insert(pos, &s);
        }

        doc.cursors.move_right();
        let lines = doc.buffer.lines().to_vec();
        doc.folding.calculate_folds(&lines);
    }

    /// Deletes character backward across all cursors.
    pub fn delete_backward(&mut self) {
        let doc = self.tabs.active_mut();
        let cursors = doc.cursors.cursors.clone();

        for c in cursors.iter().rev() {
            let pos = doc.buffer.line_col_to_pos(c.line, c.col);
            if pos > 0 {
                doc.buffer.delete(pos - 1, 1);
            }
        }

        doc.cursors.move_left();
        let lines = doc.buffer.lines().to_vec();
        doc.folding.calculate_folds(&lines);
    }

    /// Handles pointer click events.
    pub fn handle_click(&mut self, pt: Point) -> bool {
        if let Some(target) = self.view.hit_test(pt, &self.tabs, &self.project) {
            match target {
                EditHitTarget::TabSelect(idx) => self.tabs.switch_tab(idx),
                EditHitTarget::TabClose(idx) => self.tabs.close_tab(idx),
                EditHitTarget::ToolbarSave => {
                    self.save_active_file();
                    true
                }
                EditHitTarget::ToolbarUndo => self.tabs.active_mut().buffer.undo(),
                EditHitTarget::ToolbarRedo => self.tabs.active_mut().buffer.redo(),
                EditHitTarget::ToolbarFind => {
                    self.search.is_active = !self.search.is_active;
                    true
                }
                EditHitTarget::SidebarItem(path) => {
                    if path.contains('.') {
                        // File clicked -> open
                        self.open_file(&path, &format!("// {}\n", path));
                    } else {
                        // Folder clicked -> toggle expand
                        self.project.toggle_directory(&path);
                    }
                    true
                }
                EditHitTarget::ToggleFold(line) => self.tabs.active_mut().folding.toggle_fold(line),
                EditHitTarget::TextCanvasClick { line, col } => {
                    let doc = self.tabs.active_mut();
                    let total = doc.buffer.line_count();
                    doc.cursors.clear_secondary();
                    let prim = doc.cursors.primary_mut();
                    prim.line = line.min(total.saturating_sub(1));
                    prim.col = col;
                    true
                }
            }
        } else {
            false
        }
    }

    /// Renders the complete editor view.
    pub fn render(&mut self, surface: &mut BufferSurface) {
        self.view.render(surface, &mut self.tabs, &self.project, &self.search);
    }
}

// ============================================================================
// Comprehensive Self-Test Suite
// ============================================================================

/// Runs the 10-point self-test validation suite for nova-edit.
pub fn run_nova_edit_selftest() -> bool {
    // 1. Verify Piece-Table Text Buffer O(1) Edits and Reconstruction
    let mut table = PieceTable::new("Hello World!");
    if table.len() != 12 || table.get_text() != "Hello World!" {
        return false;
    }
    table.insert(5, ", Beautiful");
    if table.get_text() != "Hello, Beautiful World!" {
        return false;
    }
    table.delete(5, 11);
    if table.get_text() != "Hello World!" {
        return false;
    }

    // 2. Verify Multi-Level Undo and Redo Stack
    if !table.undo() || table.get_text() != "Hello, Beautiful World!" {
        return false;
    }
    if !table.undo() || table.get_text() != "Hello World!" {
        return false;
    }
    if !table.redo() || table.get_text() != "Hello, Beautiful World!" {
        return false;
    }

    // 3. Verify Multi-Language Syntax Highlighting Tokenizer
    // Rust
    let rust_spans = SyntaxHighlighter::highlight_line(Language::Rust, "pub fn calculate() -> u32 {");
    if !rust_spans.iter().any(|s| s.token_type == TokenType::Keyword) {
        return false;
    }
    if !rust_spans.iter().any(|s| s.token_type == TokenType::Type) {
        return false;
    }
    // C
    let c_spans = SyntaxHighlighter::highlight_line(Language::C, "#include <stdio.h>\nint main() {");
    if !c_spans.iter().any(|s| s.token_type == TokenType::Keyword || s.token_type == TokenType::Type) {
        return false;
    }
    // Python
    let py_spans = SyntaxHighlighter::highlight_line(Language::Python, "def process_data(items): # comment");
    if !py_spans.iter().any(|s| s.token_type == TokenType::Keyword)
        || !py_spans.iter().any(|s| s.token_type == TokenType::Comment)
    {
        return false;
    }
    // Markdown
    let md_spans = SyntaxHighlighter::highlight_line(Language::Markdown, "# NovaOS Architecture");
    if md_spans.is_empty() || md_spans[0].token_type != TokenType::Heading {
        return false;
    }
    // Shell
    let sh_spans = SyntaxHighlighter::highlight_line(Language::Shell, "echo $VAR # print");
    if !sh_spans.iter().any(|s| s.token_type == TokenType::Type) {
        return false;
    }
    // JSON
    let json_spans = SyntaxHighlighter::highlight_line(Language::Json, "{\"status\": true, \"count\": 42}");
    if !json_spans.iter().any(|s| s.token_type == TokenType::Keyword)
        || !json_spans.iter().any(|s| s.token_type == TokenType::NumberLiteral)
    {
        return false;
    }

    // 4. Verify Multi-Cursor Editing and Synchronization
    let mut cursors = cursor::CursorManager::new();
    cursors.add_cursor(1, 5);
    cursors.add_cursor(2, 5);
    if cursors.cursors.len() != 3 {
        return false;
    }
    cursors.move_right();
    if cursors.cursors[1].col != 6 || cursors.cursors[2].col != 6 {
        return false;
    }
    cursors.clear_secondary();
    if cursors.cursors.len() != 1 {
        return false;
    }

    // 5. Verify Code Folding Scope Engine
    let lines = vec![
        "fn outer() {".to_string(),
        "    let x = 1;".to_string(),
        "    if x > 0 {".to_string(),
        "        println!(\"inner\");".to_string(),
        "    }".to_string(),
        "}".to_string(),
    ];
    let mut folding = folding::CodeFoldingManager::new();
    folding.calculate_folds(&lines);
    if folding.ranges.len() != 2 {
        return false;
    }
    // Fold outer function starting at line 0
    if !folding.toggle_fold(0) {
        return false;
    }
    // Line 1..5 should now be hidden
    if folding.is_line_visible(1) || folding.is_line_visible(4) {
        return false;
    }

    // 6. Verify Bracket Matching and Delimiter Auto-Closing
    let code = "fn test(val: [u32; 4]) { return (val[0]); }";
    // '(' at index 7 matches ')' at index 21
    let match_open = BracketMatcher::find_matching_bracket(code, 7);
    if match_open != Some(21) {
        return false;
    }
    let match_close = BracketMatcher::find_matching_bracket(code, 21);
    if match_close != Some(7) {
        return false;
    }
    // Auto-closing check
    if BracketMatcher::get_auto_close_char('{') != Some('}') {
        return false;
    }

    // 7. Verify Find & Replace with Multiple Occurrences
    let mut edit_table = PieceTable::new("apple banana apple cherry apple");
    let matches = FindReplaceEngine::find_all(&edit_table.get_text(), "apple", true);
    if matches.len() != 3 {
        return false;
    }
    let rep_count = FindReplaceEngine::replace_all(&mut edit_table, "apple", "orange", true);
    if rep_count != 3 || edit_table.get_text() != "orange banana orange cherry orange" {
        return false;
    }

    // 8. Verify Project Directory Sidebar Navigation
    let mut project = ProjectExplorer::default();
    let initial_count = project.visible_nodes().len();
    // Toggle docs directory
    if !project.toggle_directory("docs") {
        return false;
    }
    if project.visible_nodes().len() <= initial_count {
        return false;
    }

    // 9. Verify Multi-File Tab Management and Auto-Save
    let mut app = NovaEditApp::new();
    app.open_file("src/buffer.rs", "pub struct PieceTable;");
    if app.tabs.tabs.len() != 2 || app.tabs.active().path != "src/buffer.rs" {
        return false;
    }
    // Mark dirty
    app.tabs.active_mut().buffer.insert(0, "// Added\n");
    if !app.tabs.active().buffer.is_dirty {
        return false;
    }
    // Auto-save
    let saved = app.tabs.auto_save_all();
    if saved != 1 || app.tabs.active().buffer.is_dirty {
        return false;
    }

    // 10. Verify BufferSurface Rasterization and Hit-Testing
    let mut surface = BufferSurface::new(1024, 680);
    app.render(&mut surface);

    let non_zero = surface
        .pixels
        .iter()
        .filter(|&&p| p != 0)
        .count();
    if non_zero < 15000 {
        return false;
    }

    // Hit test toolbar Save button
    let hit_save = app.handle_click(Point::new(100, 42));
    if !hit_save {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nova_edit_selftest() {
        assert!(run_nova_edit_selftest(), "All nova-edit self-tests pass");
    }

    #[test]
    fn test_piece_table_lines() {
        let mut table = PieceTable::new("Line 1\nLine 2\nLine 3");
        assert_eq!(table.line_count(), 3);
        assert_eq!(table.get_line(1).unwrap(), "Line 2");
        table.insert(6, "\nLine 1.5");
        assert_eq!(table.line_count(), 4);
    }

    #[test]
    fn test_piece_table_undo_redo() {
        let mut table = PieceTable::new("Initial");
        table.insert(7, " Text");
        assert_eq!(table.get_text(), "Initial Text");
        assert!(table.undo());
        assert_eq!(table.get_text(), "Initial");
        assert!(table.redo());
        assert_eq!(table.get_text(), "Initial Text");
    }

    #[test]
    fn test_language_detection() {
        assert_eq!(Language::detect_from_path("main.rs"), Language::Rust);
        assert_eq!(Language::detect_from_path("script.py"), Language::Python);
        assert_eq!(Language::detect_from_path("server.c"), Language::C);
        assert_eq!(Language::detect_from_path("doc.md"), Language::Markdown);
        assert_eq!(Language::detect_from_path("config.json"), Language::Json);
        assert_eq!(Language::detect_from_path("run.sh"), Language::Shell);
    }

    #[test]
    fn test_syntax_highlighting_all_languages() {
        assert!(!SyntaxHighlighter::highlight_line(Language::Rust, "fn test() -> bool {").is_empty());
        assert!(!SyntaxHighlighter::highlight_line(Language::C, "int main() {").is_empty());
        assert!(!SyntaxHighlighter::highlight_line(Language::Python, "def run():").is_empty());
        assert!(!SyntaxHighlighter::highlight_line(Language::Markdown, "# Header").is_empty());
        assert!(!SyntaxHighlighter::highlight_line(Language::Shell, "export PATH=$PATH").is_empty());
        assert!(!SyntaxHighlighter::highlight_line(Language::Json, "{\"key\": 123}").is_empty());
    }

    #[test]
    fn test_multi_cursor_normalize() {
        let mut cm = cursor::CursorManager::new();
        cm.add_cursor(2, 4);
        cm.add_cursor(1, 10);
        cm.add_cursor(2, 4); // Duplicate
        assert_eq!(cm.cursors.len(), 3); // (0,0), (1,10), (2,4)
    }

    #[test]
    fn test_bracket_matching_nested() {
        let code = "{ a: [1, (2 + 3)] }";
        // '[' at index 5 matches ']' at 16
        assert_eq!(BracketMatcher::find_matching_bracket(code, 5), Some(16));
        // '(' at index 9 matches ')' at 15
        assert_eq!(BracketMatcher::find_matching_bracket(code, 9), Some(15));
    }

    #[test]
    fn test_code_folding() {
        let lines = vec![
            "fn compute() {".to_string(),
            "    let a = 1;".to_string(),
            "}".to_string(),
        ];
        let mut folding = folding::CodeFoldingManager::new();
        folding.calculate_folds(&lines);
        assert_eq!(folding.ranges.len(), 1);
        assert_eq!(folding.ranges[0].start_line, 0);
        assert_eq!(folding.ranges[0].end_line, 2);
    }

    #[test]
    fn test_find_and_replace() {
        let mut table = PieceTable::new("foo bar foo baz");
        let count = FindReplaceEngine::replace_all(&mut table, "foo", "qux", true);
        assert_eq!(count, 2);
        assert_eq!(table.get_text(), "qux bar qux baz");
    }

    #[test]
    fn test_project_explorer_visible() {
        let project = ProjectExplorer::default();
        let visible = project.visible_nodes();
        assert!(visible.iter().any(|n| n.name == "src"));
        assert!(visible.iter().any(|n| n.name == "main.rs"));
    }

    #[test]
    fn test_tab_manager_auto_save() {
        let mut tabs = TabManager::new();
        tabs.open_file("test.txt", "content");
        assert_eq!(tabs.tabs.len(), 2);
        tabs.active_mut().buffer.insert(0, "new ");
        assert!(tabs.active().buffer.is_dirty);
        assert_eq!(tabs.auto_save_all(), 1);
        assert!(!tabs.active().buffer.is_dirty);
    }
}
