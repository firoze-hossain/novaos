//! userland/desktop/apps/novash/src/syntax.rs
//!
//! Real-Time Syntax Highlighter for NovaShell (novash) inspired by Fish and Zsh.
//! Classifies tokens on the fly into commands, flags, strings, variables, operators,
//! and comments, applying ANSI TrueColor/16-color styling.

#[cfg(not(test))]
use alloc::{string::String, vec::Vec};
#[cfg(test)]
use std::{string::String, vec::Vec};

/// Token category for syntax highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    ValidCommand,
    UnknownCommand,
    OptionFlag,
    StringLiteral,
    Variable,
    Operator,
    Number,
    FilePath,
    Comment,
    Argument,
    Whitespace,
}

/// A parsed syntax token with its span and category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxToken {
    pub token_type: TokenType,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Real-time syntax highlighter.
pub struct SyntaxHighlighter {
    pub known_commands: Vec<String>,
}

impl SyntaxHighlighter {
    pub fn new() -> Self {
        let mut highlighter = Self {
            known_commands: Vec::new(),
        };

        // Populate standard built-in and default NovaOS system commands
        let defaults = [
            "cd", "pwd", "echo", "exit", "history", "help", "export", "unset",
            "alias", "unalias", "source", "bookmark", "b", "j", "pushd", "popd",
            "dirs", "jobs", "fg", "bg", "true", "false", "test", "clear", "type",
            "which", "ls", "cat", "cp", "mv", "rm", "mkdir", "rmdir", "touch",
            "grep", "find", "head", "tail", "wc", "sort", "uniq", "chmod", "ps",
            "kill", "htop", "uname", "df", "free", "git", "cargo", "rustc", "nova",
            "novash", "edit", "files", "term",
        ];

        for &cmd in &defaults {
            highlighter.known_commands.push(String::from(cmd));
        }

        highlighter
    }

    /// Register an extra known command or alias
    pub fn register_command(&mut self, cmd: &str) {
        if !self.is_known_command(cmd) {
            self.known_commands.push(String::from(cmd));
        }
    }

    /// Check if a command is recognized
    pub fn is_known_command(&self, cmd: &str) -> bool {
        self.known_commands.iter().any(|k| k == cmd)
    }

    /// Tokenize the raw command line into syntax tokens
    pub fn tokenize(&self, input: &str) -> Vec<SyntaxToken> {
        let mut tokens = Vec::new();
        let chars: Vec<char> = input.chars().collect();
        let len = chars.len();
        let mut i = 0;
        let mut expecting_command = true;

        while i < len {
            // 1. Whitespace
            if chars[i].is_whitespace() {
                let start = i;
                while i < len && chars[i].is_whitespace() {
                    i += 1;
                }
                tokens.push(SyntaxToken {
                    token_type: TokenType::Whitespace,
                    start,
                    end: i,
                    text: chars[start..i].iter().collect(),
                });
                continue;
            }

            // 2. Comments (# to end of line)
            if chars[i] == '#' {
                let start = i;
                i = len;
                tokens.push(SyntaxToken {
                    token_type: TokenType::Comment,
                    start,
                    end: i,
                    text: chars[start..i].iter().collect(),
                });
                break;
            }

            // 3. String literals ('...' or "...")
            if chars[i] == '\'' || chars[i] == '"' {
                let quote = chars[i];
                let start = i;
                i += 1;
                while i < len && chars[i] != quote {
                    if chars[i] == '\\' && i + 1 < len {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if i < len && chars[i] == quote {
                    i += 1;
                }
                tokens.push(SyntaxToken {
                    token_type: TokenType::StringLiteral,
                    start,
                    end: i,
                    text: chars[start..i].iter().collect(),
                });
                expecting_command = false;
                continue;
            }

            // 4. Operators: |, ||, &&, ;, &, >, >>, <, 2>, 2>&1
            if chars[i] == '|' || chars[i] == '&' || chars[i] == ';' || chars[i] == '>' || chars[i] == '<' {
                let start = i;
                let c = chars[i];
                if i + 1 < len && chars[i + 1] == c {
                    i += 2;
                } else if c == '>' && i + 1 < len && chars[i + 1] == '>' {
                    i += 2;
                } else {
                    i += 1;
                }
                tokens.push(SyntaxToken {
                    token_type: TokenType::Operator,
                    start,
                    end: i,
                    text: chars[start..i].iter().collect(),
                });
                expecting_command = true; // After an operator, a new command starts
                continue;
            }

            // 5. Variables ($VAR or ${VAR})
            if chars[i] == '$' {
                let start = i;
                i += 1;
                if i < len && chars[i] == '{' {
                    while i < len && chars[i] != '}' {
                        i += 1;
                    }
                    if i < len && chars[i] == '}' {
                        i += 1;
                    }
                } else {
                    while i < len && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '?') {
                        i += 1;
                    }
                }
                tokens.push(SyntaxToken {
                    token_type: TokenType::Variable,
                    start,
                    end: i,
                    text: chars[start..i].iter().collect(),
                });
                expecting_command = false;
                continue;
            }

            // 6. Generic word / identifier
            let start = i;
            while i < len && !chars[i].is_whitespace() && chars[i] != '|' && chars[i] != '&' && chars[i] != ';' && chars[i] != '>' && chars[i] != '<' {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();

            let token_type = if expecting_command {
                expecting_command = false;
                if self.is_known_command(&word) || word.starts_with("./") || word.starts_with('/') {
                    TokenType::ValidCommand
                } else {
                    TokenType::UnknownCommand
                }
            } else if word.starts_with('-') {
                TokenType::OptionFlag
            } else if word.chars().all(|c| c.is_ascii_digit()) {
                TokenType::Number
            } else if word.contains('/') || word.starts_with('.') || word.starts_with('~') {
                TokenType::FilePath
            } else {
                TokenType::Argument
            };

            tokens.push(SyntaxToken {
                token_type,
                start,
                end: i,
                text: word,
            });
        }

        tokens
    }

    /// Render syntax-highlighted ANSI string for terminal display
    pub fn highlight(&self, input: &str) -> String {
        let tokens = self.tokenize(input);
        let mut output = String::new();

        for token in tokens {
            let color = match token.token_type {
                TokenType::ValidCommand => "\x1b[32m",        // Green
                TokenType::UnknownCommand => "\x1b[31m",      // Red
                TokenType::OptionFlag => "\x1b[36m",          // Cyan
                TokenType::StringLiteral => "\x1b[33m",       // Yellow
                TokenType::Variable => "\x1b[94m",            // Bright Blue
                TokenType::Operator => "\x1b[35m",            // Magenta
                TokenType::Number => "\x1b[95m",              // Bright Magenta
                TokenType::FilePath => "\x1b[97m",            // Bright White
                TokenType::Comment => "\x1b[90m",             // Dim Gray
                TokenType::Argument => "\x1b[0m",             // Default
                TokenType::Whitespace => "\x1b[0m",           // Default
            };

            output.push_str(color);
            output.push_str(&token.text);
            output.push_str("\x1b[0m");
        }

        output
    }
}
