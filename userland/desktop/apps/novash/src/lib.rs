//! userland/desktop/apps/novash/src/lib.rs
//!
//! # NovaShell (novash) - Modern Interactive Rust Shell for NovaOS
//!
//! Next-generation interactive user shell inspired by Fish and Zsh with:
//! - Real-time syntax highlighting for commands, options, strings, variables, and operators
//! - History-based autosuggestions with inline ghost text and word-by-word acceptance
//! - Context-aware tab completions for built-ins, binaries, file paths, and subcommands
//! - Rich interactive prompt with git branch detection, dirty status, and execution timers
//! - Directory bookmarks (`bookmark`/`b`/`j`) and directory stack (`pushd`/`popd`/`dirs`)
//! - POSIX pipelines (`|`), redirections (`>`, `>>`, `<`, `2>`, `2>&1`), and chaining (`&&`, `||`, `;`)
//! - Scripting and control flow engine (`if`/`then`/`else`, `for`/`in`, `while`, `test`/`[`)
//! - Interactive line editor based on rustyline/liner concepts with full ANSI redraw

#![cfg_attr(not(test), no_std)]
#![allow(dead_code, unused_variables)]

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::{
    format,
    string::String,
    vec,
    vec::Vec,
};

#[cfg(test)]
extern crate std;
#[cfg(test)]
use std::{
    format,
    string::String,
    vec,
    vec::Vec,
};

pub mod bookmarks;
pub mod completion;
pub mod executor;
pub mod history;
pub mod line_reader;
pub mod parser;
pub mod prompt;
pub mod script;
pub mod syntax;

use bookmarks::BookmarkManager;
use completion::CompletionEngine;
use executor::{CommandExecutor, ExecutionResult, ShellEnvironment};
use history::CommandHistory;
use line_reader::{KeyAction, LineReader};
use parser::CommandParser;
use prompt::{GitStatus, PromptContext, PromptRenderer};
use script::{ConditionEvaluator, ScriptEngine};
use syntax::{SyntaxHighlighter, TokenType};

/// Top-level shell orchestrator.
pub struct NovaShell {
    pub reader: LineReader,
    pub env: ShellEnvironment,
    pub is_running: bool,
}

impl NovaShell {
    pub fn new() -> Self {
        let mut shell = Self {
            reader: LineReader::new(),
            env: ShellEnvironment::new(),
            is_running: true,
        };

        // Sync prompt with environment
        shell.sync_prompt();
        shell
    }

    /// Synchronize prompt context with current shell environment
    pub fn sync_prompt(&mut self) {
        self.reader.prompt.user = self.env.get_var("USER").unwrap_or_else(|| String::from("root"));
        self.reader.prompt.hostname = self.env.get_var("HOSTNAME").unwrap_or_else(|| String::from("novaos"));
        self.reader.prompt.cwd = self.env.cwd.clone();
        self.reader.prompt.home_dir = self.env.home.clone();
        self.reader.prompt.last_exit_code = self.env.last_exit_code;
    }

    /// Execute a single command line string
    pub fn execute_line(&mut self, line: &str) -> ExecutionResult {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return ExecutionResult::success("");
        }

        // Add to history
        self.reader.history.add(trimmed);

        // Check for alias replacement on first word
        let mut effective_line = String::from(trimmed);
        if let Some(first_space) = trimmed.find(' ') {
            let cmd = &trimmed[..first_space];
            if let Some(alias_val) = self.env.get_alias(cmd) {
                effective_line = format!("{} {}", alias_val, &trimmed[first_space + 1..]);
            }
        } else if let Some(alias_val) = self.env.get_alias(trimmed) {
            effective_line = String::from(alias_val);
        }

        // Parse command line into AST
        let ast = match CommandParser::parse(&effective_line) {
            Ok(ast) => ast,
            Err(err) => {
                let err_msg = format!("novash: {}\n", err);
                self.env.last_exit_code = 1;
                self.sync_prompt();
                return ExecutionResult::failure(1, &err_msg);
            }
        };

        // Execute AST
        let result = CommandExecutor::execute_ast(&ast, &mut self.env);
        self.sync_prompt();
        result
    }

    /// Execute a multi-line script string
    pub fn run_script(&mut self, script: &str) -> i32 {
        let mut last_code = 0;
        for line in script.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let res = self.execute_line(trimmed);
            last_code = res.exit_code;
        }
        last_code
    }
}

/// Comprehensive selftest runner for NovaShell (novash).
/// Verifies all 10 core subsystems:
/// 1. Syntax highlighting & tokenization (valid vs invalid commands, flags, strings, vars)
/// 2. Command history & Fish-style autosuggestions (prefix match, suffix ghost text, word acceptance)
/// 3. Tab completion engine (builtins, binaries, git subcommands, file paths)
/// 4. Git branch & status prompt (HEAD parsing, branch name, dirty flag, exit indicators)
/// 5. Directory bookmarks & directory stack (b/j, pushd/popd/dirs)
/// 6. POSIX AST parser & pipelines (pipes, redirections, conditional chaining)
/// 7. Variable & tilde expansion ($VAR, $?, ~)
/// 8. Scripting control flow (test/[, for loops, condition evaluation)
/// 9. Built-in command execution (cd, pwd, echo, export, alias)
/// 10. Interactive line editor (keystrokes, cursor navigation, ghost text blitting)
pub fn run_novash_selftest() -> bool {
    // 1. Syntax Highlighter & Tokenizer Verification
    let syntax = SyntaxHighlighter::new();
    let tokens = syntax.tokenize("echo -n 'hello world' $USER | grep error # comment");

    if tokens.is_empty() {
        return false;
    }
    // Verify first command is recognized as valid
    if tokens[0].token_type != TokenType::ValidCommand || tokens[0].text != "echo" {
        return false;
    }
    // Verify unknown command turns red/UnknownCommand
    let unk_tokens = syntax.tokenize("nonexistent_binary_xyz");
    if unk_tokens[0].token_type != TokenType::UnknownCommand {
        return false;
    }
    // Verify highlighting produces ANSI escape codes
    let hl = syntax.highlight("ls -la /home");
    if !hl.contains("\x1b[32m") || !hl.contains("\x1b[36m") {
        return false;
    }

    // 2. Command History & Fish Autosuggestions
    let mut history = CommandHistory::new();
    history.add("git checkout -b feature/kernel-cow");
    history.add("cargo build --release");

    let sugg = history.suggest("git che");
    if let Some(s) = sugg {
        if s.full_match != "git checkout -b feature/kernel-cow" {
            return false;
        }
        if s.suffix != "ckout -b feature/kernel-cow" {
            return false;
        }
        // Verify word-by-word suggestion acceptance
        let word = CommandHistory::next_suggestion_word(&s.suffix);
        if word != "ckout " {
            return false;
        }
    } else {
        return false;
    }

    // 3. Tab Completion Engine
    let completion = CompletionEngine::new();
    let fs_entries = vec![
        String::from("Cargo.toml"),
        String::from("README.md"),
        String::from("src/"),
        String::from("kernel/"),
    ];
    let bookmarks = vec![String::from("home"), String::from("docs")];

    // Complete builtin
    let c1 = completion.complete("ec", 2, &fs_entries, &bookmarks);
    if !c1.matches.iter().any(|m| m.value == "echo") {
        return false;
    }

    // Complete git subcommand
    let c2 = completion.complete("git comm", 8, &fs_entries, &bookmarks);
    if !c2.matches.iter().any(|m| m.value == "commit") {
        return false;
    }

    // Complete directory
    let c3 = completion.complete("cd sr", 5, &fs_entries, &bookmarks);
    if !c3.matches.iter().any(|m| m.value == "src/") {
        return false;
    }

    // 4. Git Branch & Status Prompt
    let head_text = "ref: refs/heads/master\n";
    let git = GitStatus::parse_head_content(head_text, true);
    if let Some(g) = git {
        if g.branch != "master" || !g.is_dirty {
            return false;
        }
    } else {
        return false;
    }

    let mut prompt_ctx = PromptContext::new();
    prompt_ctx.git = Some(GitStatus::new("master", true));
    prompt_ctx.cwd = String::from("/home/firoze/projects");
    prompt_ctx.home_dir = String::from("/home/firoze");
    if prompt_ctx.display_cwd() != "~/projects" {
        return false;
    }
    let (rendered_prompt, _) = PromptRenderer::render(&prompt_ctx);
    if !rendered_prompt.contains("master") || !rendered_prompt.contains('*') || !rendered_prompt.contains("➜") {
        return false;
    }

    // 5. Directory Bookmarks & Stack
    let mut bm = BookmarkManager::new();
    bm.add("proj", "/home/firoze/projects/others/novaos");
    if bm.get("proj") != Some("/home/firoze/projects/others/novaos") {
        return false;
    }
    if bm.resolve("@proj") != Some(String::from("/home/firoze/projects/others/novaos")) {
        return false;
    }
    // Directory stack
    let new_dir = bm.pushd("/home/firoze", "/tmp").unwrap();
    if new_dir != "/tmp" {
        return false;
    }
    let popped = bm.popd().unwrap();
    if popped != "/home/firoze" {
        return false;
    }

    // 6. POSIX AST & Pipelines Parser
    let parsed = CommandParser::parse("cat input.txt | grep error > output.log 2>&1");
    if let Ok(ast) = parsed {
        if ast.pipelines.len() != 1 {
            return false;
        }
        let pipeline = &ast.pipelines[0].0;
        if pipeline.commands.len() != 2 {
            return false;
        }
        if pipeline.commands[0].name != "cat" || pipeline.commands[1].name != "grep" {
            return false;
        }
        if pipeline.commands[1].redirects.len() != 2 {
            return false;
        }
    } else {
        return false;
    }

    // 7. Variable & Tilde Expansion
    let expanded = CommandParser::expand_variables(
        "~/files/$USER/code_${STATUS}",
        |k| match k {
            "USER" => Some(String::from("developer")),
            "STATUS" => Some(String::from("active")),
            _ => None,
        },
        0,
        "/home/developer",
    );
    if expanded != "/home/developer/files/developer/code_active" {
        return false;
    }

    // 8. Scripting & Control Flow
    assert!(ConditionEvaluator::evaluate(&[
        String::from("["),
        String::from("5"),
        String::from("-eq"),
        String::from("5"),
        String::from("]")
    ]));
    assert!(ConditionEvaluator::evaluate(&[
        String::from("test"),
        String::from("-n"),
        String::from("non-empty")
    ]));

    let mut for_results = Vec::new();
    ScriptEngine::execute_for_loop(
        "file",
        &[String::from("a.txt"), String::from("b.txt")],
        &[String::from("echo $file")],
        |cmd| for_results.push(String::from(cmd)),
    );
    if for_results.len() != 2 || for_results[0] != "echo a.txt" || for_results[1] != "echo b.txt" {
        return false;
    }

    // 9. Built-in Command Execution
    let mut shell = NovaShell::new();
    let res1 = shell.execute_line("export TEST_VAR=NovaOS_Fast");
    if res1.exit_code != 0 || shell.env.get_var("TEST_VAR").as_deref() != Some("NovaOS_Fast") {
        return false;
    }

    let res2 = shell.execute_line("echo $TEST_VAR");
    if !res2.stdout.contains("NovaOS_Fast") {
        return false;
    }

    let res3 = shell.execute_line("cd ~");
    if res3.exit_code != 0 || shell.env.cwd != shell.env.home {
        return false;
    }

    // 10. Interactive Line Reader
    let mut reader = LineReader::new();
    reader.handle_key(KeyAction::Char('e'), &[], &[]);
    reader.handle_key(KeyAction::Char('c'), &[], &[]);
    reader.handle_key(KeyAction::Char('h'), &[], &[]);
    reader.handle_key(KeyAction::Char('o'), &[], &[]);
    if reader.buffer != "echo" || reader.cursor != 4 {
        return false;
    }
    reader.handle_key(KeyAction::Backspace, &[], &[]);
    if reader.buffer != "ech" || reader.cursor != 3 {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_novash_selftest() {
        assert!(run_novash_selftest(), "NovaShell selftest failed!");
    }

    #[test]
    fn test_syntax_highlighting() {
        let syntax = SyntaxHighlighter::new();
        let hl = syntax.highlight("git status --short");
        assert!(hl.contains("\x1b[32m")); // Green for git
        assert!(hl.contains("\x1b[36m")); // Cyan for --short
    }

    #[test]
    fn test_autosuggestion_full_and_partial() {
        let mut history = CommandHistory::new();
        history.add("cargo test --all");

        let sugg = history.suggest("cargo t").expect("Should find suggestion");
        assert_eq!(sugg.full_match, "cargo test --all");
        assert_eq!(sugg.suffix, "est --all");

        let word = CommandHistory::next_suggestion_word(&sugg.suffix);
        assert_eq!(word, "est ");
    }

    #[test]
    fn test_tab_completion_subcommands() {
        let completion = CompletionEngine::new();
        let res = completion.complete("cargo bu", 8, &[], &[]);
        assert!(res.matches.iter().any(|m| m.value == "build"));
    }

    #[test]
    fn test_git_prompt_parsing() {
        let head = "ref: refs/heads/feature/novash-modern\n";
        let status = GitStatus::parse_head_content(head, false).unwrap();
        assert_eq!(status.branch, "feature/novash-modern");
        assert!(!status.is_dirty);
    }

    #[test]
    fn test_bookmarks_and_dirs() {
        let mut bm = BookmarkManager::new();
        bm.add("kernel", "/home/firoze/projects/novaos/kernel");
        assert_eq!(bm.resolve("@kernel"), Some(String::from("/home/firoze/projects/novaos/kernel")));

        let _ = bm.pushd("/a", "/b");
        assert_eq!(bm.popd(), Ok(String::from("/a")));
    }

    #[test]
    fn test_pipeline_execution() {
        let mut shell = NovaShell::new();
        let res = shell.execute_line("echo pipeline_test | cat");
        assert_eq!(res.exit_code, 0);
        assert!(res.stdout.contains("pipeline_test"));
    }

    #[test]
    fn test_control_flow_condition() {
        assert!(ConditionEvaluator::evaluate(&[
            String::from("["),
            String::from("hello"),
            String::from("="),
            String::from("hello"),
            String::from("]")
        ]));
        assert!(!ConditionEvaluator::evaluate(&[
            String::from("["),
            String::from("10"),
            String::from("-lt"),
            String::from("5"),
            String::from("]")
        ]));
    }

    #[test]
    fn test_line_reader_editing() {
        let mut reader = LineReader::new();
        reader.insert_str("nova");
        assert_eq!(reader.buffer, "nova");
        reader.move_left();
        reader.insert_char('-');
        assert_eq!(reader.buffer, "nov-a");
        reader.move_home();
        assert_eq!(reader.cursor, 0);
        reader.move_end();
        assert_eq!(reader.cursor, 5);
    }

    #[test]
    fn test_env_variable_expansion_and_export() {
        let mut shell = NovaShell::new();
        shell.execute_line("export GREETING=Hello_World");
        assert_eq!(shell.env.get_var("GREETING"), Some(String::from("Hello_World")));
        let res = shell.execute_line("echo $GREETING");
        assert!(res.stdout.contains("Hello_World"));
    }

    #[test]
    fn test_interactive_ghost_suggestion_rendering() {
        let mut reader = LineReader::new();
        reader.history.add("git status");
        reader.insert_str("git s");
        assert!(reader.active_suggestion.is_some());
        let display = reader.render_display();
        assert!(display.contains("\x1b[90m")); // Contains dim ghost text
        assert!(display.contains("tatus"));
    }

    #[test]
    fn test_script_execution_multi_line() {
        let mut shell = NovaShell::new();
        let script = "export COUNT=1\ncd ~\npwd\n";
        let exit_code = shell.run_script(script);
        assert_eq!(exit_code, 0);
        assert_eq!(shell.env.get_var("COUNT"), Some(String::from("1")));
        assert_eq!(shell.env.cwd, shell.env.home);
    }
}
