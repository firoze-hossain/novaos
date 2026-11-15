//! userland/desktop/apps/novash/src/parser.rs
//!
//! POSIX Shell Lexer, AST Parser, Pipelines & Redirections for NovaShell.
//! Supports quotes, environment variable expansion, redirection operators (`>`, `>>`, `<`, `2>`, `2>&1`),
//! multi-stage pipelines (`|`), and conditional chaining (`&&`, `||`, `;`, `&`).

#[cfg(not(test))]
use alloc::{string::String, string::ToString, vec::Vec};
#[cfg(test)]
use std::{string::String, string::ToString, vec::Vec};

/// Redirection target and type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectKind {
    StdoutOverwrite, // >
    StdoutAppend,    // >>
    Stdin,           // <
    StderrOverwrite, // 2>
    StderrAppend,    // 2>>
    StderrToStdout,  // 2>&1
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirection {
    pub kind: RedirectKind,
    pub target: String,
}

/// A single simple command with arguments and redirections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleCommand {
    pub name: String,
    pub args: Vec<String>,
    pub redirects: Vec<Redirection>,
    pub env_vars: Vec<(String, String)>,
}

impl SimpleCommand {
    pub fn new(name: &str) -> Self {
        Self {
            name: String::from(name),
            args: Vec::new(),
            redirects: Vec::new(),
            env_vars: Vec::new(),
        }
    }
}

/// A pipeline of commands connected via standard Unix pipes (`|`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    pub commands: Vec<SimpleCommand>,
    pub is_background: bool,
}

/// Logical chain operator between pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainOp {
    None,
    And,      // &&
    Or,       // ||
    Sequence, // ;
}

/// Complete parsed command line consisting of chained pipelines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLineAst {
    pub pipelines: Vec<(Pipeline, ChainOp)>,
}

/// Parser for shell input commands.
pub struct CommandParser;

impl CommandParser {
    /// Expands variables like `$VAR`, `${VAR}`, `$?`, `$$`, and `~`
    pub fn expand_variables<F>(input: &str, get_var: F, last_exit: i32, home: &str) -> String
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut result = String::new();
        let chars: Vec<char> = input.chars().collect();
        let len = chars.len();
        let mut i = 0;

        // Tilde expansion at start of token
        if chars.starts_with(&['~']) && (chars.len() == 1 || chars[1] == '/') {
            result.push_str(home);
            i = 1;
        }

        while i < len {
            if chars[i] == '$' && i + 1 < len {
                i += 1;
                if chars[i] == '?' {
                    result.push_str(&last_exit.to_string());
                    i += 1;
                } else if chars[i] == '$' {
                    result.push_str("1001"); // Shell PID
                    i += 1;
                } else if chars[i] == '{' {
                    i += 1;
                    let start = i;
                    while i < len && chars[i] != '}' {
                        i += 1;
                    }
                    let var_name: String = chars[start..i].iter().collect();
                    if i < len && chars[i] == '}' {
                        i += 1;
                    }
                    if let Some(val) = get_var(&var_name) {
                        result.push_str(&val);
                    }
                } else if chars[i].is_alphanumeric() || chars[i] == '_' {
                    let start = i;
                    while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
                        i += 1;
                    }
                    let var_name: String = chars[start..i].iter().collect();
                    if let Some(val) = get_var(&var_name) {
                        result.push_str(&val);
                    }
                } else {
                    result.push('$');
                }
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }

        result
    }

    /// Tokenizes command line respecting single and double quotes
    pub fn tokenize_words(input: &str) -> Vec<String> {
        let mut words = Vec::new();
        let chars: Vec<char> = input.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            // Skip leading whitespace
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }
            if i >= len {
                break;
            }

            let mut current = String::new();
            while i < len && !chars[i].is_whitespace() {
                if chars[i] == '\'' {
                    // Single quotes: preserve all characters literally
                    i += 1;
                    while i < len && chars[i] != '\'' {
                        current.push(chars[i]);
                        i += 1;
                    }
                    if i < len && chars[i] == '\'' {
                        i += 1;
                    }
                } else if chars[i] == '"' {
                    // Double quotes: parse characters (variables expanded later)
                    i += 1;
                    while i < len && chars[i] != '"' {
                        if chars[i] == '\\' && i + 1 < len {
                            i += 1;
                            current.push(chars[i]);
                            i += 1;
                        } else {
                            current.push(chars[i]);
                            i += 1;
                        }
                    }
                    if i < len && chars[i] == '"' {
                        i += 1;
                    }
                } else if chars[i] == '\\' && i + 1 < len {
                    i += 1;
                    current.push(chars[i]);
                    i += 1;
                } else {
                    current.push(chars[i]);
                    i += 1;
                }
            }

            if !current.is_empty() {
                words.push(current);
            }
        }

        words
    }

    /// Parses command line string into a full CommandLineAst
    pub fn parse(input: &str) -> Result<CommandLineAst, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(CommandLineAst {
                pipelines: Vec::new(),
            });
        }

        let words = Self::tokenize_words(trimmed);
        if words.is_empty() {
            return Ok(CommandLineAst {
                pipelines: Vec::new(),
            });
        }

        let mut pipelines = Vec::new();
        let mut current_pipeline = Pipeline {
            commands: Vec::new(),
            is_background: false,
        };
        let mut current_cmd: Option<SimpleCommand> = None;
        let mut i = 0;

        while i < words.len() {
            let w = &words[i];

            match w.as_str() {
                "|" => {
                    // Pipeline pipe
                    if let Some(cmd) = current_cmd.take() {
                        current_pipeline.commands.push(cmd);
                    } else {
                        return Err(String::from("Syntax error: unexpected pipe '|'"));
                    }
                    i += 1;
                }
                "&&" => {
                    if let Some(cmd) = current_cmd.take() {
                        current_pipeline.commands.push(cmd);
                    }
                    pipelines.push((current_pipeline, ChainOp::And));
                    current_pipeline = Pipeline {
                        commands: Vec::new(),
                        is_background: false,
                    };
                    i += 1;
                }
                "||" => {
                    if let Some(cmd) = current_cmd.take() {
                        current_pipeline.commands.push(cmd);
                    }
                    pipelines.push((current_pipeline, ChainOp::Or));
                    current_pipeline = Pipeline {
                        commands: Vec::new(),
                        is_background: false,
                    };
                    i += 1;
                }
                ";" => {
                    if let Some(cmd) = current_cmd.take() {
                        current_pipeline.commands.push(cmd);
                    }
                    pipelines.push((current_pipeline, ChainOp::Sequence));
                    current_pipeline = Pipeline {
                        commands: Vec::new(),
                        is_background: false,
                    };
                    i += 1;
                }
                "&" => {
                    if let Some(cmd) = current_cmd.take() {
                        current_pipeline.commands.push(cmd);
                    }
                    current_pipeline.is_background = true;
                    pipelines.push((current_pipeline, ChainOp::Sequence));
                    current_pipeline = Pipeline {
                        commands: Vec::new(),
                        is_background: false,
                    };
                    i += 1;
                }
                ">" | "1>" => {
                    i += 1;
                    if i < words.len() {
                        let target = words[i].clone();
                        if let Some(ref mut cmd) = current_cmd {
                            cmd.redirects.push(Redirection {
                                kind: RedirectKind::StdoutOverwrite,
                                target,
                            });
                        }
                        i += 1;
                    } else {
                        return Err(String::from("Syntax error: missing target for '>'"));
                    }
                }
                ">>" | "1>>" => {
                    i += 1;
                    if i < words.len() {
                        let target = words[i].clone();
                        if let Some(ref mut cmd) = current_cmd {
                            cmd.redirects.push(Redirection {
                                kind: RedirectKind::StdoutAppend,
                                target,
                            });
                        }
                        i += 1;
                    } else {
                        return Err(String::from("Syntax error: missing target for '>>'"));
                    }
                }
                "<" => {
                    i += 1;
                    if i < words.len() {
                        let target = words[i].clone();
                        if let Some(ref mut cmd) = current_cmd {
                            cmd.redirects.push(Redirection {
                                kind: RedirectKind::Stdin,
                                target,
                            });
                        }
                        i += 1;
                    } else {
                        return Err(String::from("Syntax error: missing target for '<'"));
                    }
                }
                "2>" => {
                    i += 1;
                    if i < words.len() {
                        let target = words[i].clone();
                        if let Some(ref mut cmd) = current_cmd {
                            cmd.redirects.push(Redirection {
                                kind: RedirectKind::StderrOverwrite,
                                target,
                            });
                        }
                        i += 1;
                    } else {
                        return Err(String::from("Syntax error: missing target for '2>'"));
                    }
                }
                "2>>" => {
                    i += 1;
                    if i < words.len() {
                        let target = words[i].clone();
                        if let Some(ref mut cmd) = current_cmd {
                            cmd.redirects.push(Redirection {
                                kind: RedirectKind::StderrAppend,
                                target,
                            });
                        }
                        i += 1;
                    } else {
                        return Err(String::from("Syntax error: missing target for '2>>'"));
                    }
                }
                "2>&1" => {
                    if let Some(ref mut cmd) = current_cmd {
                        cmd.redirects.push(Redirection {
                            kind: RedirectKind::StderrToStdout,
                            target: String::from("&1"),
                        });
                    }
                    i += 1;
                }
                _ => {
                    // Regular argument or command name
                    if current_cmd.is_none() {
                        // Check for preceding environment variable assignment: KEY=VALUE
                        if let Some(eq_idx) = w.find('=') {
                            if eq_idx > 0 && !w.starts_with('-') {
                                let key = w[..eq_idx].to_string();
                                let val = w[eq_idx + 1..].to_string();
                                let mut cmd = SimpleCommand::new("");
                                cmd.env_vars.push((key, val));
                                current_cmd = Some(cmd);
                                i += 1;
                                continue;
                            }
                        }
                        let mut cmd = SimpleCommand::new(w);
                        cmd.args.push(w.clone());
                        current_cmd = Some(cmd);
                    } else {
                        let cmd = current_cmd.as_mut().unwrap();
                        if cmd.name.is_empty() {
                            cmd.name = w.clone();
                        }
                        cmd.args.push(w.clone());
                    }
                    i += 1;
                }
            }
        }

        if let Some(cmd) = current_cmd.take() {
            current_pipeline.commands.push(cmd);
        }
        if !current_pipeline.commands.is_empty() {
            pipelines.push((current_pipeline, ChainOp::None));
        }

        Ok(CommandLineAst { pipelines })
    }
}
