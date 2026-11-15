//! userland/desktop/apps/novash/src/script.rs
//!
//! Scripting and Control Flow Engine for NovaShell (novash).
//! Implements `if`/`then`/`else`/`fi`, `for`/`in`/`do`/`done`, `while`/`do`/`done`,
//! and conditional expression evaluation (`test` / `[`).

#[cfg(not(test))]
use alloc::{format, string::String, vec::Vec};
#[cfg(test)]
use std::{format, string::String, vec::Vec};

/// Evaluates conditional expressions matching POSIX `test` / `[` ... `]`.
pub struct ConditionEvaluator;

impl ConditionEvaluator {
    /// Evaluates condition arguments. Returns true (exit 0) or false (exit 1).
    pub fn evaluate(args: &[String]) -> bool {
        let mut slice = args;
        // Strip leading '[' and trailing ']' if present
        if let Some(first) = slice.first() {
            if first == "[" || first == "test" {
                slice = &slice[1..];
            }
        }
        if let Some(last) = slice.last() {
            if last == "]" {
                slice = &slice[..slice.len() - 1];
            }
        }

        match slice.len() {
            0 => false,
            1 => !slice[0].is_empty(),
            2 => {
                match slice[0].as_str() {
                    "-z" => slice[1].is_empty(),
                    "-n" => !slice[1].is_empty(),
                    "-f" | "-e" => {
                        // In simulated/selftest mode, check common files or non-empty path
                        !slice[1].is_empty() && (slice[1].contains('.') || slice[1].starts_with('/'))
                    }
                    "-d" => slice[1].ends_with('/') || slice[1].starts_with('/') || slice[1] == ".",
                    "!" => !Self::evaluate(&slice[1..]),
                    _ => false,
                }
            }
            3 => {
                let op = slice[1].as_str();
                let lhs = slice[0].as_str();
                let rhs = slice[2].as_str();

                match op {
                    "=" | "==" => lhs == rhs,
                    "!=" => lhs != rhs,
                    "-eq" => {
                        let a: i64 = lhs.parse().unwrap_or(0);
                        let b: i64 = rhs.parse().unwrap_or(0);
                        a == b
                    }
                    "-ne" => {
                        let a: i64 = lhs.parse().unwrap_or(0);
                        let b: i64 = rhs.parse().unwrap_or(0);
                        a != b
                    }
                    "-lt" => {
                        let a: i64 = lhs.parse().unwrap_or(0);
                        let b: i64 = rhs.parse().unwrap_or(0);
                        a < b
                    }
                    "-gt" => {
                        let a: i64 = lhs.parse().unwrap_or(0);
                        let b: i64 = rhs.parse().unwrap_or(0);
                        a > b
                    }
                    "-le" => {
                        let a: i64 = lhs.parse().unwrap_or(0);
                        let b: i64 = rhs.parse().unwrap_or(0);
                        a <= b
                    }
                    "-ge" => {
                        let a: i64 = lhs.parse().unwrap_or(0);
                        let b: i64 = rhs.parse().unwrap_or(0);
                        a >= b
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }
}

/// Control flow statement block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlStatement {
    If {
        condition: Vec<String>,
        then_branch: Vec<String>,
        else_branch: Option<Vec<String>>,
    },
    For {
        var_name: String,
        items: Vec<String>,
        body: Vec<String>,
    },
    While {
        condition: Vec<String>,
        body: Vec<String>,
    },
}

/// Script block executor.
pub struct ScriptEngine;

impl ScriptEngine {
    /// Detects if line begins a structured block (if / for / while)
    pub fn is_control_start(line: &str) -> bool {
        let first = line.split_whitespace().next().unwrap_or("");
        first == "if" || first == "for" || first == "while"
    }

    /// Parses and executes a single for-loop in-memory:
    /// `for item in a b c; do echo $item; done`
    pub fn execute_for_loop<F>(
        var_name: &str,
        items: &[String],
        body_template: &[String],
        mut execute_cmd: F,
    ) where
        F: FnMut(&str),
    {
        for item in items {
            for cmd in body_template {
                let replaced = cmd.replace(&format!("${}", var_name), item);
                execute_cmd(&replaced);
            }
        }
    }
}
