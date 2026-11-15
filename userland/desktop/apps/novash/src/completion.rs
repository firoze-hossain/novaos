//! userland/desktop/apps/novash/src/completion.rs
//!
//! Intelligent Tab Completion Engine for NovaShell (novash) inspired by Fish and Zsh.
//! Provides context-aware completions for built-in commands, external binaries,
//! file system paths, command flags, git subcommands, and directory bookmarks.

#[cfg(not(test))]
use alloc::{format, string::String, string::ToString, vec::Vec};
#[cfg(test)]
use std::{format, string::String, string::ToString, vec::Vec};

/// A single completion candidate with metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub value: String,
    pub display: String,
    pub description: String,
    pub is_directory: bool,
}

impl CompletionItem {
    pub fn new(value: &str, description: &str) -> Self {
        Self {
            value: String::from(value),
            display: String::from(value),
            description: String::from(description),
            is_directory: false,
        }
    }

    pub fn directory(value: &str) -> Self {
        Self {
            value: String::from(value),
            display: String::from(value),
            description: String::from("directory"),
            is_directory: true,
        }
    }
}

/// The result of a tab-completion query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionResult {
    pub replacement_start: usize,
    pub replacement_end: usize,
    pub matches: Vec<CompletionItem>,
    pub common_prefix: Option<String>,
}

/// Context-aware tab completion engine.
pub struct CompletionEngine {
    pub builtins: Vec<String>,
    pub known_binaries: Vec<String>,
}

impl CompletionEngine {
    pub fn new() -> Self {
        let mut engine = Self {
            builtins: Vec::new(),
            known_binaries: Vec::new(),
        };

        let default_builtins = [
            ("cd", "Change working directory"),
            ("pwd", "Print current working directory"),
            ("echo", "Write arguments to output"),
            ("exit", "Exit the shell"),
            ("history", "View or clear command history"),
            ("help", "Display shell built-in help"),
            ("export", "Set environment variables"),
            ("unset", "Unset environment variables"),
            ("alias", "Create command shortcuts"),
            ("unalias", "Remove command shortcuts"),
            ("source", "Execute commands from a script"),
            ("bookmark", "Manage directory bookmarks"),
            ("b", "Quick bookmark add/list"),
            ("j", "Jump to bookmarked directory"),
            ("pushd", "Push directory onto stack"),
            ("popd", "Pop directory from stack"),
            ("dirs", "Display directory stack"),
            ("jobs", "List background active jobs"),
            ("fg", "Bring job to foreground"),
            ("bg", "Send job to background"),
            ("clear", "Clear terminal screen"),
            ("type", "Display information about command type"),
            ("which", "Locate a command executable"),
        ];

        for &(cmd, _) in &default_builtins {
            engine.builtins.push(String::from(cmd));
        }

        let default_bins = [
            "ls", "cat", "cp", "mv", "rm", "mkdir", "rmdir", "touch", "grep",
            "find", "head", "tail", "wc", "sort", "uniq", "chmod", "ps", "kill",
            "htop", "uname", "df", "free", "git", "cargo", "rustc", "nova",
            "novash", "edit", "files", "term", "curl", "ping", "tar",
        ];

        for &bin in &default_bins {
            engine.known_binaries.push(String::from(bin));
        }

        engine
    }

    /// Query completions for the command line at cursor position.
    pub fn complete(
        &self,
        line: &str,
        cursor_pos: usize,
        fs_entries: &[String],
        bookmarks: &[String],
    ) -> CompletionResult {
        let cursor = cursor_pos.min(line.len());
        let line_prefix = &line[..cursor];

        // Find the start of the current word being typed
        let word_start = line_prefix
            .rfind(|c: char| c.is_whitespace() || c == '|' || c == '&' || c == ';')
            .map(|idx| idx + 1)
            .unwrap_or(0);

        let word = &line_prefix[word_start..];
        let mut candidates = Vec::new();

        // Determine context: is this the first word of the command or an argument?
        let before_word = line_prefix[..word_start].trim();
        let is_command_position = before_word.is_empty()
            || before_word.ends_with('|')
            || before_word.ends_with('&')
            || before_word.ends_with(';');

        if is_command_position {
            // 1. Complete Built-ins
            for b in &self.builtins {
                if b.starts_with(word) {
                    candidates.push(CompletionItem::new(b, "built-in command"));
                }
            }

            // 2. Complete Known System Binaries
            for bin in &self.known_binaries {
                if bin.starts_with(word) && !candidates.iter().any(|c| c.value == *bin) {
                    candidates.push(CompletionItem::new(bin, "executable binary"));
                }
            }

            // 3. Complete Executable local paths (./...)
            if word.starts_with('.') || word.starts_with('/') {
                self.complete_file_paths(word, fs_entries, &mut candidates, false);
            }
        } else {
            // Argument completion context
            let words_before: Vec<&str> = before_word.split_whitespace().collect();
            let prev_cmd = words_before.last().copied().unwrap_or("");

            match prev_cmd {
                "cd" | "pushd" | "rmdir" => {
                    // Directory only completion
                    self.complete_file_paths(word, fs_entries, &mut candidates, true);
                }
                "j" | "bookmark" => {
                    // Directory bookmark completion
                    for bm in bookmarks {
                        if bm.starts_with(word) {
                            candidates.push(CompletionItem::new(bm, "directory bookmark"));
                        }
                    }
                }
                "git" => {
                    // Git subcommands
                    let git_subcmds = [
                        ("status", "Show working tree status"),
                        ("checkout", "Switch branches or restore files"),
                        ("branch", "List, create, or delete branches"),
                        ("commit", "Record changes to repository"),
                        ("diff", "Show changes between commits"),
                        ("log", "Show commit logs"),
                        ("push", "Update remote refs along with objects"),
                        ("pull", "Fetch and integrate with remote"),
                        ("add", "Add file contents to the index"),
                        ("clone", "Clone a repository into a directory"),
                        ("stash", "Stash changes in dirty directory"),
                        ("merge", "Join two or more development histories"),
                        ("rebase", "Reapply commits on top of another base"),
                    ];
                    for &(sc, desc) in &git_subcmds {
                        if sc.starts_with(word) {
                            candidates.push(CompletionItem::new(sc, desc));
                        }
                    }
                }
                "cargo" => {
                    // Cargo subcommands
                    let cargo_subcmds = [
                        ("build", "Compile the current package"),
                        ("check", "Analyze the current package and report errors"),
                        ("test", "Execute unit and integration tests"),
                        ("run", "Run a binary or example of the local package"),
                        ("clean", "Remove generated artifacts"),
                        ("new", "Create a new cargo package"),
                        ("init", "Create a new cargo package in an existing directory"),
                        ("update", "Update dependencies listed in Cargo.lock"),
                        ("tree", "Display a tree visualization of dependency graph"),
                    ];
                    for &(sc, desc) in &cargo_subcmds {
                        if sc.starts_with(word) {
                            candidates.push(CompletionItem::new(sc, desc));
                        }
                    }
                }
                _ => {
                    // General file path completion
                    self.complete_file_paths(word, fs_entries, &mut candidates, false);
                }
            }
        }

        // Compute common prefix across all matches
        let common_prefix = if candidates.len() > 1 {
            let first = &candidates[0].value;
            let mut prefix_len = first.len();
            for c in &candidates[1..] {
                let match_len = first
                    .chars()
                    .zip(c.value.chars())
                    .take_while(|(a, b)| a == b)
                    .count();
                if match_len < prefix_len {
                    prefix_len = match_len;
                }
            }
            if prefix_len > word.len() {
                Some(first[..prefix_len].to_string())
            } else {
                None
            }
        } else {
            None
        };

        CompletionResult {
            replacement_start: word_start,
            replacement_end: cursor,
            matches: candidates,
            common_prefix,
        }
    }

    /// Match and populate file path candidates
    fn complete_file_paths(
        &self,
        word: &str,
        fs_entries: &[String],
        candidates: &mut Vec<CompletionItem>,
        dirs_only: bool,
    ) {
        for entry in fs_entries {
            let is_dir = entry.ends_with('/');
            if dirs_only && !is_dir {
                continue;
            }

            if entry.starts_with(word) {
                let item = if is_dir {
                    CompletionItem::directory(entry)
                } else {
                    CompletionItem::new(entry, "file")
                };
                candidates.push(item);
            }
        }
    }

    /// Format multiple completions into a multi-column terminal pager layout
    pub fn format_pager(matches: &[CompletionItem], selected_idx: Option<usize>) -> Vec<String> {
        let mut lines = Vec::new();
        if matches.is_empty() {
            return lines;
        }

        let max_val_len = matches.iter().map(|m| m.display.len()).max().unwrap_or(10);
        let col_width = (max_val_len + 4).max(16);

        // Display up to 10 entries cleanly
        for (idx, item) in matches.iter().enumerate().take(12) {
            let is_sel = selected_idx == Some(idx);
            let indicator = if is_sel { "\x1b[7m" } else { "" };
            let reset = if is_sel { "\x1b[0m" } else { "" };

            let type_color = if item.is_directory {
                "\x1b[1;34m" // Bold blue for directory
            } else {
                "\x1b[0m"
            };

            let line = format!(
                "  {}{}{:width$}\x1b[0m  \x1b[90m({}){}\x1b[0m",
                indicator,
                type_color,
                item.display,
                item.description,
                reset,
                width = col_width
            );
            lines.push(line);
        }

        if matches.len() > 12 {
            lines.push(format!("  \x1b[90m... and {} more\x1b[0m", matches.len() - 12));
        }

        lines
    }
}
