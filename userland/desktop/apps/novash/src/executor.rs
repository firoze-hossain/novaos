//! userland/desktop/apps/novash/src/executor.rs
//!
//! Command Execution Engine, Built-in Dispatcher & Pipeline Executor for NovaShell.
//! Manages environment variables, aliases, current working directory, directory stack,
//! and pipe data streams.

#[cfg(not(test))]
use alloc::{format, string::String, string::ToString, vec::Vec};
#[cfg(test)]
use std::{format, string::String, string::ToString, vec::Vec};

use crate::bookmarks::BookmarkManager;
use crate::parser::{ChainOp, CommandLineAst, CommandParser, Pipeline, RedirectKind, SimpleCommand};
use crate::script::ConditionEvaluator;

/// Output result of executing a command or pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ExecutionResult {
    pub fn success(stdout: &str) -> Self {
        Self {
            exit_code: 0,
            stdout: String::from(stdout),
            stderr: String::new(),
        }
    }

    pub fn failure(code: i32, stderr: &str) -> Self {
        Self {
            exit_code: code,
            stdout: String::new(),
            stderr: String::from(stderr),
        }
    }
}

/// The runtime shell environment.
pub struct ShellEnvironment {
    pub env_vars: Vec<(String, String)>,
    pub aliases: Vec<(String, String)>,
    pub cwd: String,
    pub prev_cwd: String,
    pub home: String,
    pub last_exit_code: i32,
    pub bookmarks: BookmarkManager,
}

impl ShellEnvironment {
    pub fn new() -> Self {
        let mut env = Self {
            env_vars: Vec::new(),
            aliases: Vec::new(),
            cwd: String::from("/home/firoze/projects/others/novaos"),
            prev_cwd: String::from("/home/firoze"),
            home: String::from("/home/firoze"),
            last_exit_code: 0,
            bookmarks: BookmarkManager::new(),
        };

        // Standard default environment
        env.set_var("USER", "root");
        env.set_var("HOSTNAME", "novaos");
        env.set_var("HOME", "/home/firoze");
        env.set_var("PATH", "/bin:/usr/bin:/sbin:/usr/local/bin");
        env.set_var("SHELL", "/bin/novash");
        env.set_var("TERM", "xterm-256color");

        // Standard aliases
        env.set_alias("ll", "ls -l");
        env.set_alias("la", "ls -la");
        env.set_alias("g", "git");

        env
    }

    pub fn get_var(&self, key: &str) -> Option<String> {
        self.env_vars
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }

    pub fn set_var(&mut self, key: &str, val: &str) {
        if let Some(existing) = self.env_vars.iter_mut().find(|(k, _)| k == key) {
            existing.1 = val.to_string();
        } else {
            self.env_vars.push((key.to_string(), val.to_string()));
        }
    }

    pub fn unset_var(&mut self, key: &str) -> bool {
        if let Some(pos) = self.env_vars.iter().position(|(k, _)| k == key) {
            self.env_vars.remove(pos);
            true
        } else {
            false
        }
    }

    pub fn set_alias(&mut self, name: &str, value: &str) {
        if let Some(existing) = self.aliases.iter_mut().find(|(k, _)| k == name) {
            existing.1 = value.to_string();
        } else {
            self.aliases.push((name.to_string(), value.to_string()));
        }
    }

    pub fn get_alias(&self, name: &str) -> Option<&str> {
        self.aliases
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Change directory handling ~, -, and bookmarks
    pub fn change_dir(&mut self, target: Option<&str>) -> Result<(), String> {
        let path = match target {
            None => self.home.clone(),
            Some("~") => self.home.clone(),
            Some("-") => self.prev_cwd.clone(),
            Some(p) => {
                // Check directory bookmarks
                if let Some(bm_path) = self.bookmarks.resolve(p) {
                    bm_path
                } else if p.starts_with("~/") {
                    format!("{}{}", self.home, &p[1..])
                } else if p.starts_with('/') {
                    p.to_string()
                } else {
                    format!("{}/{}", self.cwd.trim_end_matches('/'), p)
                }
            }
        };

        self.prev_cwd = self.cwd.clone();
        self.cwd = path;
        Ok(())
    }
}

/// Execution dispatcher.
pub struct CommandExecutor;

impl CommandExecutor {
    /// Execute a full AST of chained pipelines
    pub fn execute_ast(ast: &CommandLineAst, env: &mut ShellEnvironment) -> ExecutionResult {
        let mut last_result = ExecutionResult::success("");

        for (pipeline, op) in &ast.pipelines {
            let res = Self::execute_pipeline(pipeline, env);
            let code = res.exit_code;
            last_result = res;
            env.last_exit_code = code;

            match op {
                ChainOp::And => {
                    if code != 0 {
                        break; // Stop on error
                    }
                }
                ChainOp::Or => {
                    if code == 0 {
                        break; // Stop on success
                    }
                }
                ChainOp::Sequence | ChainOp::None => {}
            }
        }

        last_result
    }

    /// Execute a multi-stage pipeline connecting stdout to stdin
    pub fn execute_pipeline(pipeline: &Pipeline, env: &mut ShellEnvironment) -> ExecutionResult {
        if pipeline.commands.is_empty() {
            return ExecutionResult::success("");
        }

        let mut current_stdin = String::new();
        let mut final_result = ExecutionResult::success("");

        for cmd in &pipeline.commands {
            let mut res = Self::execute_simple_command(cmd, &current_stdin, env);

            // Process output redirections if specified
            for r in &cmd.redirects {
                match r.kind {
                    RedirectKind::StdoutOverwrite | RedirectKind::StdoutAppend => {
                        // In simulation / shell environment, record redirected output
                        res.stdout.clear();
                    }
                    _ => {}
                }
            }

            current_stdin = res.stdout.clone();
            final_result = res;
        }

        final_result
    }

    /// Execute a single command (builtin or external)
    pub fn execute_simple_command(
        cmd: &SimpleCommand,
        stdin_input: &str,
        env: &mut ShellEnvironment,
    ) -> ExecutionResult {
        // Expand any environment variables in command arguments
        let expanded_args: Vec<String> = cmd
            .args
            .iter()
            .map(|arg| {
                CommandParser::expand_variables(
                    arg,
                    |k| env.get_var(k),
                    env.last_exit_code,
                    &env.home,
                )
            })
            .collect();

        let cmd_name = if let Some(first) = expanded_args.first() {
            first.as_str()
        } else {
            return ExecutionResult::success("");
        };

        // 1. Built-in command dispatch
        match cmd_name {
            "cd" => {
                let target = expanded_args.get(1).map(|s| s.as_str());
                match env.change_dir(target) {
                    Ok(_) => ExecutionResult::success(""),
                    Err(e) => ExecutionResult::failure(1, &e),
                }
            }
            "pwd" => {
                let out = format!("{}\n", env.cwd);
                ExecutionResult::success(&out)
            }
            "echo" => {
                let args_slice = if expanded_args.len() > 1 {
                    &expanded_args[1..]
                } else {
                    &[]
                };
                let out = format!("{}\n", args_slice.join(" "));
                ExecutionResult::success(&out)
            }
            "export" => {
                if expanded_args.len() > 1 {
                    for arg in &expanded_args[1..] {
                        if let Some(eq) = arg.find('=') {
                            let k = &arg[..eq];
                            let v = &arg[eq + 1..];
                            env.set_var(k, v);
                        }
                    }
                }
                ExecutionResult::success("")
            }
            "unset" => {
                if expanded_args.len() > 1 {
                    for arg in &expanded_args[1..] {
                        env.unset_var(arg);
                    }
                }
                ExecutionResult::success("")
            }
            "env" => {
                let mut out = String::new();
                for (k, v) in &env.env_vars {
                    out.push_str(&format!("{}={}\n", k, v));
                }
                ExecutionResult::success(&out)
            }
            "alias" => {
                if expanded_args.len() == 1 {
                    let mut out = String::new();
                    for (k, v) in &env.aliases {
                        out.push_str(&format!("alias {}='{}'\n", k, v));
                    }
                    ExecutionResult::success(&out)
                } else {
                    for arg in &expanded_args[1..] {
                        if let Some(eq) = arg.find('=') {
                            let k = &arg[..eq];
                            let v = &arg[eq + 1..];
                            env.set_alias(k, v);
                        }
                    }
                    ExecutionResult::success("")
                }
            }
            "unalias" => {
                if expanded_args.len() > 1 {
                    for arg in &expanded_args[1..] {
                        env.aliases.retain(|(k, _)| k != arg);
                    }
                }
                ExecutionResult::success("")
            }
            "bookmark" | "b" => {
                if expanded_args.len() == 1 || expanded_args.get(1).map(|s| s.as_str()) == Some("list") {
                    let mut out = String::new();
                    for b in &env.bookmarks.bookmarks {
                        out.push_str(&format!("{} -> {}\n", b.name, b.path));
                    }
                    ExecutionResult::success(&out)
                } else if expanded_args.get(1).map(|s| s.as_str()) == Some("add") {
                    let name = expanded_args.get(2).map(|s| s.as_str()).unwrap_or("");
                    let path = expanded_args.get(3).map(|s| s.as_str()).unwrap_or(&env.cwd);
                    if !name.is_empty() {
                        env.bookmarks.add(name, path);
                    }
                    ExecutionResult::success("")
                } else if expanded_args.get(1).map(|s| s.as_str()) == Some("del") {
                    let name = expanded_args.get(2).map(|s| s.as_str()).unwrap_or("");
                    env.bookmarks.remove(name);
                    ExecutionResult::success("")
                } else {
                    let name = expanded_args[1].as_str();
                    let path = expanded_args.get(2).map(|s| s.as_str()).unwrap_or(&env.cwd);
                    env.bookmarks.add(name, path);
                    ExecutionResult::success("")
                }
            }
            "j" => {
                if let Some(name) = expanded_args.get(1) {
                    if let Some(p) = env.bookmarks.get(name) {
                        let path = p.to_string();
                        let _ = env.change_dir(Some(&path));
                        ExecutionResult::success("")
                    } else {
                        ExecutionResult::failure(1, &format!("Bookmark '{}' not found\n", name))
                    }
                } else {
                    ExecutionResult::failure(1, "Usage: j <bookmark_name>\n")
                }
            }
            "pushd" => {
                if let Some(dir) = expanded_args.get(1) {
                    let cur = env.cwd.clone();
                    if let Ok(new_dir) = env.bookmarks.pushd(&cur, dir) {
                        let _ = env.change_dir(Some(&new_dir));
                    }
                    let out = env.bookmarks.list_dirs(&env.cwd).join(" ");
                    ExecutionResult::success(&format!("{}\n", out))
                } else {
                    ExecutionResult::failure(1, "Usage: pushd <directory>\n")
                }
            }
            "popd" => {
                match env.bookmarks.popd() {
                    Ok(new_dir) => {
                        let _ = env.change_dir(Some(&new_dir));
                        let out = env.bookmarks.list_dirs(&env.cwd).join(" ");
                        ExecutionResult::success(&format!("{}\n", out))
                    }
                    Err(e) => ExecutionResult::failure(1, &format!("popd: {}\n", e)),
                }
            }
            "dirs" => {
                let out = env.bookmarks.list_dirs(&env.cwd).join("\n");
                ExecutionResult::success(&format!("{}\n", out))
            }
            "test" | "[" => {
                let passed = ConditionEvaluator::evaluate(&expanded_args);
                if passed {
                    ExecutionResult::success("")
                } else {
                    ExecutionResult::failure(1, "")
                }
            }
            "true" => ExecutionResult::success(""),
            "false" => ExecutionResult::failure(1, ""),
            "exit" => {
                let code: i32 = expanded_args
                    .get(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                ExecutionResult::failure(code, "exit\n")
            }
            "help" => {
                let help_text = "NovaShell (novash) v2.0.0 - Built-in Commands:\n\
                                 cd, pwd, echo, export, unset, env, alias, unalias,\n\
                                 bookmark, b, j, pushd, popd, dirs, test, [, true, false, exit, help\n";
                ExecutionResult::success(help_text)
            }
            "clear" => ExecutionResult::success("\x1b[2J\x1b[H"),
            _ => {
                // External binary or pipeline simulation
                if !stdin_input.is_empty() {
                    // Filter or passthrough stream
                    ExecutionResult::success(stdin_input)
                } else {
                    let msg = format!("novash: executed: {}\n", expanded_args.join(" "));
                    ExecutionResult::success(&msg)
                }
            }
        }
    }
}
