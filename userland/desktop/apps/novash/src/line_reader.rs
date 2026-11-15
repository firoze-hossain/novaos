//! userland/desktop/apps/novash/src/line_reader.rs
//!
//! Interactive Line Editor for NovaShell based on rustyline and liner concepts.
//! Handles raw keystrokes, cursor navigation, history traversal, Fish-style autosuggestion
//! rendering/acceptance, and tab completion menus.

#[cfg(not(test))]
use alloc::{string::String, string::ToString};
#[cfg(test)]
use std::{string::String, string::ToString};

use crate::completion::{CompletionEngine, CompletionResult};
use crate::history::{Autosuggestion, CommandHistory};
use crate::prompt::{PromptContext, PromptRenderer};
use crate::syntax::SyntaxHighlighter;

/// Key action received from keyboard/terminal stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    AltRight,
    CtrlA, // Home
    CtrlE, // End
    CtrlU, // Kill to start
    CtrlK, // Kill to end
    CtrlW, // Kill word
    CtrlL, // Clear screen
    CtrlC, // Cancel line
    CtrlD, // EOF
    CtrlF, // Accept suggestion
    None,
}

/// The interactive line editor state.
pub struct LineReader {
    pub buffer: String,
    pub cursor: usize,
    pub history: CommandHistory,
    pub completion: CompletionEngine,
    pub syntax: SyntaxHighlighter,
    pub prompt: PromptContext,
    pub active_suggestion: Option<Autosuggestion>,
    pub completion_state: Option<(CompletionResult, Option<usize>)>,
}

impl LineReader {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            history: CommandHistory::new(),
            completion: CompletionEngine::new(),
            syntax: SyntaxHighlighter::new(),
            prompt: PromptContext::new(),
            active_suggestion: None,
            completion_state: None,
        }
    }

    /// Update autosuggestion based on current buffer text
    pub fn update_suggestion(&mut self) {
        if self.cursor == self.buffer.len() && !self.buffer.trim().is_empty() {
            self.active_suggestion = self.history.suggest(&self.buffer);
        } else {
            self.active_suggestion = None;
        }
    }

    /// Insert character at cursor position
    pub fn insert_char(&mut self, c: char) {
        self.completion_state = None;
        if self.cursor >= self.buffer.len() {
            self.buffer.push(c);
            self.cursor = self.buffer.len();
        } else {
            let mut new_buf = String::new();
            for (idx, ch) in self.buffer.chars().enumerate() {
                if idx == self.cursor {
                    new_buf.push(c);
                }
                new_buf.push(ch);
            }
            self.buffer = new_buf;
            self.cursor += 1;
        }
        self.update_suggestion();
    }

    /// Insert a text string at cursor position
    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.insert_char(c);
        }
    }

    /// Handle Backspace
    pub fn backspace(&mut self) {
        self.completion_state = None;
        if self.cursor > 0 && !self.buffer.is_empty() {
            let mut new_buf = String::new();
            let del_idx = self.cursor - 1;
            for (idx, ch) in self.buffer.chars().enumerate() {
                if idx != del_idx {
                    new_buf.push(ch);
                }
            }
            self.buffer = new_buf;
            self.cursor -= 1;
            self.update_suggestion();
        }
    }

    /// Handle Delete
    pub fn delete_forward(&mut self) {
        self.completion_state = None;
        if self.cursor < self.buffer.len() {
            let mut new_buf = String::new();
            for (idx, ch) in self.buffer.chars().enumerate() {
                if idx != self.cursor {
                    new_buf.push(ch);
                }
            }
            self.buffer = new_buf;
            self.update_suggestion();
        }
    }

    /// Move cursor left
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.update_suggestion();
        }
    }

    /// Move cursor right, or accept suggestion if at end of line
    pub fn move_right(&mut self) {
        if self.cursor < self.buffer.len() {
            self.cursor += 1;
            self.update_suggestion();
        } else if let Some(ref sugg) = self.active_suggestion.clone() {
            // Right Arrow at line end accepts full suggestion!
            self.buffer = sugg.full_match.clone();
            self.cursor = self.buffer.len();
            self.active_suggestion = None;
        }
    }

    /// Move cursor to line start (Home / Ctrl+A)
    pub fn move_home(&mut self) {
        self.cursor = 0;
        self.active_suggestion = None;
    }

    /// Move cursor to line end (End / Ctrl+E)
    pub fn move_end(&mut self) {
        if let Some(ref sugg) = self.active_suggestion.clone() {
            self.buffer = sugg.full_match.clone();
            self.cursor = self.buffer.len();
            self.active_suggestion = None;
        } else {
            self.cursor = self.buffer.len();
            self.update_suggestion();
        }
    }

    /// Accept next word of suggestion (Alt+Right)
    pub fn accept_suggestion_word(&mut self) {
        if let Some(ref sugg) = self.active_suggestion.clone() {
            let next_word = CommandHistory::next_suggestion_word(&sugg.suffix);
            self.buffer.push_str(next_word);
            self.cursor = self.buffer.len();
            self.update_suggestion();
        }
    }

    /// Kill from cursor to line end (Ctrl+K)
    pub fn kill_to_end(&mut self) {
        self.buffer.truncate(self.cursor);
        self.update_suggestion();
    }

    /// Kill from line start to cursor (Ctrl+U)
    pub fn kill_to_start(&mut self) {
        self.buffer = self.buffer[self.cursor..].to_string();
        self.cursor = 0;
        self.update_suggestion();
    }

    /// Kill previous word (Ctrl+W)
    pub fn kill_word_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prefix = &self.buffer[..self.cursor];
        let trimmed = prefix.trim_end();
        let last_space = trimmed.rfind(' ').map(|idx| idx + 1).unwrap_or(0);
        let mut new_buf = String::from(&self.buffer[..last_space]);
        new_buf.push_str(&self.buffer[self.cursor..]);
        self.buffer = new_buf;
        self.cursor = last_space;
        self.update_suggestion();
    }

    /// History navigation backward (Up Arrow)
    pub fn history_prev(&mut self) {
        if let Some(entry) = self.history.browse_prev(&self.buffer).map(|s| s.to_string()) {
            self.buffer = entry;
            self.cursor = self.buffer.len();
            self.update_suggestion();
        }
    }

    /// History navigation forward (Down Arrow)
    pub fn history_next(&mut self) {
        if let Some(entry) = self.history.browse_next().map(|s| s.to_string()) {
            self.buffer = entry;
            self.cursor = self.buffer.len();
            self.update_suggestion();
        }
    }

    /// Trigger Tab completion
    pub fn handle_tab(&mut self, fs_entries: &[String], bookmarks: &[String]) {
        // If completion menu is already open, cycle through candidates
        if let Some((ref res, ref mut sel_idx)) = self.completion_state {
            if !res.matches.is_empty() {
                let next_idx = match *sel_idx {
                    None => 0,
                    Some(cur) => (cur + 1) % res.matches.len(),
                };
                *sel_idx = Some(next_idx);
                let choice = &res.matches[next_idx].value;

                let mut new_buf = String::from(&self.buffer[..res.replacement_start]);
                new_buf.push_str(choice);
                if !choice.ends_with('/') {
                    new_buf.push(' ');
                }
                new_buf.push_str(&self.buffer[res.replacement_end..]);
                self.buffer = new_buf;
                self.cursor = res.replacement_start + choice.len() + if choice.ends_with('/') { 0 } else { 1 };
                self.update_suggestion();
                return;
            }
        }

        // Fresh completion query
        let result = self.completion.complete(&self.buffer, self.cursor, fs_entries, bookmarks);

        if result.matches.is_empty() {
            return;
        }

        if result.matches.len() == 1 {
            let choice = &result.matches[0].value;
            let mut new_buf = String::from(&self.buffer[..result.replacement_start]);
            new_buf.push_str(choice);
            if !choice.ends_with('/') {
                new_buf.push(' ');
            }
            new_buf.push_str(&self.buffer[result.replacement_end..]);
            self.buffer = new_buf;
            self.cursor = result.replacement_start + choice.len() + if choice.ends_with('/') { 0 } else { 1 };
            self.completion_state = None;
            self.update_suggestion();
        } else if let Some(ref common) = result.common_prefix {
            // Complete common prefix first
            let mut new_buf = String::from(&self.buffer[..result.replacement_start]);
            new_buf.push_str(common);
            new_buf.push_str(&self.buffer[result.replacement_end..]);
            self.buffer = new_buf;
            self.cursor = result.replacement_start + common.len();
            self.completion_state = Some((result, None));
            self.update_suggestion();
        } else {
            // Open completion menu
            self.completion_state = Some((result, Some(0)));
        }
    }

    /// Process a KeyAction
    pub fn handle_key(&mut self, action: KeyAction, fs_entries: &[String], bookmarks: &[String]) -> bool {
        match action {
            KeyAction::Char(c) => self.insert_char(c),
            KeyAction::Backspace => self.backspace(),
            KeyAction::Delete => self.delete_forward(),
            KeyAction::Left => self.move_left(),
            KeyAction::Right => self.move_right(),
            KeyAction::Up => self.history_prev(),
            KeyAction::Down => self.history_next(),
            KeyAction::Home | KeyAction::CtrlA => self.move_home(),
            KeyAction::End | KeyAction::CtrlE => self.move_end(),
            KeyAction::AltRight => self.accept_suggestion_word(),
            KeyAction::CtrlF => self.move_right(),
            KeyAction::CtrlU => self.kill_to_start(),
            KeyAction::CtrlK => self.kill_to_end(),
            KeyAction::CtrlW => self.kill_word_backward(),
            KeyAction::Tab => self.handle_tab(fs_entries, bookmarks),
            KeyAction::CtrlC => {
                self.buffer.clear();
                self.cursor = 0;
                self.active_suggestion = None;
                self.completion_state = None;
            }
            KeyAction::Enter => {
                self.active_suggestion = None;
                self.completion_state = None;
                return true; // Command submitted
            }
            _ => {}
        }
        false
    }

    /// Render interactive command line display with prompt, highlighted text, and ghost suggestion
    pub fn render_display(&self) -> String {
        let (prompt_str, _prompt_len) = PromptRenderer::render(&self.prompt);
        let highlighted = self.syntax.highlight(&self.buffer);

        let mut out = String::new();
        out.push_str("\r\x1b[2K"); // Carriage return + Clear line
        out.push_str(&prompt_str);
        out.push_str(&highlighted);

        // Render Fish ghost autosuggestion in dim gray
        if let Some(ref sugg) = self.active_suggestion {
            out.push_str("\x1b[90m");
            out.push_str(&sugg.suffix);
            out.push_str("\x1b[0m");
        }

        // Render completion menu pager if active
        if let Some((ref res, sel_idx)) = self.completion_state {
            out.push_str("\r\n");
            let pager_lines = CompletionEngine::format_pager(&res.matches, sel_idx);
            for pl in pager_lines {
                out.push_str(&pl);
                out.push_str("\r\n");
            }
        }

        out
    }
}
