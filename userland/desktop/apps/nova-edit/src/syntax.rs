//! userland/desktop/apps/nova-edit/src/syntax.rs
//!
//! Multi-Language Syntax Highlighting Engine for NovaOS Code Editor.
//! Supports Rust, C, Python, Markdown, Shell, and JSON.

#[cfg(not(test))]
use alloc::{vec, vec::Vec};

#[cfg(test)]
use std::{vec, vec::Vec};

use nova_ui::prelude::Color;

/// Supported programming, scripting, and markup languages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    C,
    Python,
    Markdown,
    Shell,
    Json,
    PlainText,
}

impl Language {
    pub fn detect_from_path(path: &str) -> Self {
        if path.ends_with(".rs") {
            Self::Rust
        } else if path.ends_with(".c") || path.ends_with(".h") || path.ends_with(".cpp") {
            Self::C
        } else if path.ends_with(".py") {
            Self::Python
        } else if path.ends_with(".md") || path.ends_with(".markdown") {
            Self::Markdown
        } else if path.ends_with(".sh") || path.ends_with(".bash") || path.ends_with(".novash") {
            Self::Shell
        } else if path.ends_with(".json") {
            Self::Json
        } else {
            Self::PlainText
        }
    }

    pub const fn name(&self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::C => "C/C++",
            Self::Python => "Python",
            Self::Markdown => "Markdown",
            Self::Shell => "Shell",
            Self::Json => "JSON",
            Self::PlainText => "Plain Text",
        }
    }
}

/// Token classification for syntax rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenType {
    Keyword,
    Type,
    StringLiteral,
    NumberLiteral,
    Comment,
    Operator,
    Heading,
    Text,
}

impl TokenType {
    pub const fn color(&self) -> Color {
        match self {
            Self::Keyword => Color::rgb(191, 90, 242),     // Accent Purple
            Self::Type => Color::rgb(90, 200, 250),        // Bright Cyan
            Self::StringLiteral => Color::rgb(48, 209, 88),// Emerald Green
            Self::NumberLiteral => Color::rgb(255, 159, 10),// Amber Orange
            Self::Comment => Color::rgb(142, 142, 147),    // Dim Muted Gray
            Self::Operator => Color::rgb(255, 55, 95),     // Coral Pink
            Self::Heading => Color::rgb(255, 214, 10),     // Golden Yellow
            Self::Text => Color::rgb(240, 240, 245),       // Crisp White
        }
    }
}

/// A highlighted range in a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxSpan {
    pub start_col: usize,
    pub end_col: usize,
    pub token_type: TokenType,
}

pub struct SyntaxHighlighter;

impl SyntaxHighlighter {
    /// Tokenizes a single line into highlighted syntax spans.
    pub fn highlight_line(lang: Language, line: &str) -> Vec<SyntaxSpan> {
        match lang {
            Language::Rust => Self::highlight_rust(line),
            Language::C => Self::highlight_c(line),
            Language::Python => Self::highlight_python(line),
            Language::Markdown => Self::highlight_markdown(line),
            Language::Shell => Self::highlight_shell(line),
            Language::Json => Self::highlight_json(line),
            Language::PlainText => vec![SyntaxSpan {
                start_col: 0,
                end_col: line.len(),
                token_type: TokenType::Text,
            }],
        }
    }

    fn highlight_rust(line: &str) -> Vec<SyntaxSpan> {
        let mut spans = Vec::new();
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            // Comments
            if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
                spans.push(SyntaxSpan {
                    start_col: i,
                    end_col: len,
                    token_type: TokenType::Comment,
                });
                break;
            }

            // Strings
            if bytes[i] == b'"' {
                let start = i;
                i += 1;
                while i < len && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < len {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if i < len { i += 1; }
                spans.push(SyntaxSpan {
                    start_col: start,
                    end_col: i,
                    token_type: TokenType::StringLiteral,
                });
                continue;
            }

            // Numbers
            if bytes[i].is_ascii_digit() {
                let start = i;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.') {
                    i += 1;
                }
                spans.push(SyntaxSpan {
                    start_col: start,
                    end_col: i,
                    token_type: TokenType::NumberLiteral,
                });
                continue;
            }

            // Identifiers / Keywords
            if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
                let start = i;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let word = &line[start..i];
                let token_type = match word {
                    "fn" | "let" | "mut" | "pub" | "struct" | "enum" | "impl" | "use" | "mod"
                    | "match" | "if" | "else" | "return" | "for" | "in" | "while" | "loop"
                    | "break" | "continue" | "unsafe" | "async" | "await" | "const" | "static"
                    | "trait" | "type" | "as" | "where" => TokenType::Keyword,
                    "u8" | "u16" | "u32" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64"
                    | "isize" | "bool" | "char" | "str" | "String" | "Vec" | "Option"
                    | "Result" | "Some" | "None" | "Ok" | "Err" | "Self" | "Box" => TokenType::Type,
                    _ => TokenType::Text,
                };
                spans.push(SyntaxSpan {
                    start_col: start,
                    end_col: i,
                    token_type,
                });
                continue;
            }

            // Operators
            if b"+-*/%=&|!^<>?:".contains(&bytes[i]) {
                spans.push(SyntaxSpan {
                    start_col: i,
                    end_col: i + 1,
                    token_type: TokenType::Operator,
                });
                i += 1;
                continue;
            }

            // Default advance
            let start = i;
            while i < len
                && !bytes[i].is_ascii_alphanumeric()
                && bytes[i] != b'_'
                && bytes[i] != b'"'
                && !b"+-*/%=&|!^<>?:/".contains(&bytes[i])
            {
                i += 1;
            }
            if i > start {
                spans.push(SyntaxSpan {
                    start_col: start,
                    end_col: i,
                    token_type: TokenType::Text,
                });
            }
        }

        spans
    }

    fn highlight_c(line: &str) -> Vec<SyntaxSpan> {
        let mut spans = Vec::new();
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
                spans.push(SyntaxSpan { start_col: i, end_col: len, token_type: TokenType::Comment });
                break;
            }
            if bytes[i] == b'#' {
                let start = i;
                while i < len && !bytes[i].is_ascii_whitespace() { i += 1; }
                spans.push(SyntaxSpan { start_col: start, end_col: i, token_type: TokenType::Keyword });
                continue;
            }
            if bytes[i] == b'"' {
                let start = i;
                i += 1;
                while i < len && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < len { i += 2; } else { i += 1; }
                }
                if i < len { i += 1; }
                spans.push(SyntaxSpan { start_col: start, end_col: i, token_type: TokenType::StringLiteral });
                continue;
            }
            if bytes[i].is_ascii_digit() {
                let start = i;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') { i += 1; }
                spans.push(SyntaxSpan { start_col: start, end_col: i, token_type: TokenType::NumberLiteral });
                continue;
            }
            if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
                let start = i;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') { i += 1; }
                let word = &line[start..i];
                let token_type = match word {
                    "int" | "char" | "void" | "long" | "short" | "float" | "double" | "unsigned" | "signed" | "size_t" | "uint32_t" | "uint64_t" => TokenType::Type,
                    "if" | "else" | "while" | "for" | "return" | "struct" | "typedef" | "switch" | "case" | "break" | "static" | "extern" | "const" => TokenType::Keyword,
                    _ => TokenType::Text,
                };
                spans.push(SyntaxSpan { start_col: start, end_col: i, token_type });
                continue;
            }
            let start = i;
            i += 1;
            spans.push(SyntaxSpan { start_col: start, end_col: i, token_type: TokenType::Text });
        }
        spans
    }

    fn highlight_python(line: &str) -> Vec<SyntaxSpan> {
        let mut spans = Vec::new();
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            if bytes[i] == b'#' {
                spans.push(SyntaxSpan { start_col: i, end_col: len, token_type: TokenType::Comment });
                break;
            }
            if bytes[i] == b'\'' || bytes[i] == b'"' {
                let quote = bytes[i];
                let start = i;
                i += 1;
                while i < len && bytes[i] != quote {
                    if bytes[i] == b'\\' && i + 1 < len { i += 2; } else { i += 1; }
                }
                if i < len { i += 1; }
                spans.push(SyntaxSpan { start_col: start, end_col: i, token_type: TokenType::StringLiteral });
                continue;
            }
            if bytes[i].is_ascii_digit() {
                let start = i;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') { i += 1; }
                spans.push(SyntaxSpan { start_col: start, end_col: i, token_type: TokenType::NumberLiteral });
                continue;
            }
            if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
                let start = i;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') { i += 1; }
                let word = &line[start..i];
                let token_type = match word {
                    "def" | "class" | "return" | "if" | "elif" | "else" | "for" | "while"
                    | "in" | "import" | "from" | "try" | "except" | "with" | "as" | "pass"
                    | "lambda" | "yield" => TokenType::Keyword,
                    "True" | "False" | "None" | "int" | "str" | "list" | "dict" | "set" => TokenType::Type,
                    _ => TokenType::Text,
                };
                spans.push(SyntaxSpan { start_col: start, end_col: i, token_type });
                continue;
            }
            let start = i;
            i += 1;
            spans.push(SyntaxSpan { start_col: start, end_col: i, token_type: TokenType::Text });
        }
        spans
    }

    fn highlight_markdown(line: &str) -> Vec<SyntaxSpan> {
        let mut spans = Vec::new();
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            spans.push(SyntaxSpan {
                start_col: 0,
                end_col: line.len(),
                token_type: TokenType::Heading,
            });
        } else if trimmed.starts_with("```") || trimmed.starts_with('`') {
            spans.push(SyntaxSpan {
                start_col: 0,
                end_col: line.len(),
                token_type: TokenType::StringLiteral,
            });
        } else {
            spans.push(SyntaxSpan {
                start_col: 0,
                end_col: line.len(),
                token_type: TokenType::Text,
            });
        }
        spans
    }

    fn highlight_shell(line: &str) -> Vec<SyntaxSpan> {
        let mut spans = Vec::new();
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            if bytes[i] == b'#' {
                spans.push(SyntaxSpan { start_col: i, end_col: len, token_type: TokenType::Comment });
                break;
            }
            if bytes[i] == b'"' || bytes[i] == b'\'' {
                let q = bytes[i];
                let start = i;
                i += 1;
                while i < len && bytes[i] != q { i += 1; }
                if i < len { i += 1; }
                spans.push(SyntaxSpan { start_col: start, end_col: i, token_type: TokenType::StringLiteral });
                continue;
            }
            if bytes[i] == b'$' {
                let start = i;
                i += 1;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'{' || bytes[i] == b'}') {
                    i += 1;
                }
                spans.push(SyntaxSpan { start_col: start, end_col: i, token_type: TokenType::Type });
                continue;
            }
            let start = i;
            i += 1;
            spans.push(SyntaxSpan { start_col: start, end_col: i, token_type: TokenType::Text });
        }
        spans
    }

    fn highlight_json(line: &str) -> Vec<SyntaxSpan> {
        let mut spans = Vec::new();
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            if bytes[i] == b'"' {
                let start = i;
                i += 1;
                while i < len && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < len { i += 2; } else { i += 1; }
                }
                if i < len { i += 1; }
                // If followed by ':', it's a key (Keyword/Type), else StringLiteral
                let mut is_key = false;
                let mut j = i;
                while j < len && bytes[j].is_ascii_whitespace() { j += 1; }
                if j < len && bytes[j] == b':' { is_key = true; }

                spans.push(SyntaxSpan {
                    start_col: start,
                    end_col: i,
                    token_type: if is_key { TokenType::Keyword } else { TokenType::StringLiteral },
                });
                continue;
            }
            if bytes[i].is_ascii_digit() || bytes[i] == b'-' {
                let start = i;
                i += 1;
                while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b'e' || bytes[i] == b'E') {
                    i += 1;
                }
                spans.push(SyntaxSpan { start_col: start, end_col: i, token_type: TokenType::NumberLiteral });
                continue;
            }
            if bytes[i].is_ascii_alphabetic() {
                let start = i;
                while i < len && bytes[i].is_ascii_alphabetic() { i += 1; }
                let word = &line[start..i];
                let token_type = if word == "true" || word == "false" || word == "null" {
                    TokenType::Type
                } else {
                    TokenType::Text
                };
                spans.push(SyntaxSpan { start_col: start, end_col: i, token_type });
                continue;
            }
            let start = i;
            i += 1;
            spans.push(SyntaxSpan { start_col: start, end_col: i, token_type: TokenType::Text });
        }
        spans
    }
}
