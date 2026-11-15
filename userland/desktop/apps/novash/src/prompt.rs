//! userland/desktop/apps/novash/src/prompt.rs
//!
//! Rich Interactive Shell Prompt with Git Branch Status & Execution Timer.
//! Inspired by Starship, Fish, and Zsh. Displays user, hostname, path abbreviation,
//! git repository branch & dirty status, and previous command exit code.

#[cfg(not(test))]
use alloc::{format, string::String};
#[cfg(test)]
use std::{format, string::String};

/// Git repository branch state and status flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatus {
    pub branch: String,
    pub is_dirty: bool,
}

impl GitStatus {
    pub fn new(branch: &str, is_dirty: bool) -> Self {
        Self {
            branch: String::from(branch),
            is_dirty,
        }
    }

    /// Parse branch name from .git/HEAD file contents
    pub fn parse_head_content(head_text: &str, is_dirty: bool) -> Option<Self> {
        let trimmed = head_text.trim();
        if let Some(ref_path) = trimmed.strip_prefix("ref: refs/heads/") {
            Some(Self::new(ref_path.trim(), is_dirty))
        } else if trimmed.len() >= 7 {
            // Detached HEAD: display 7-char short commit SHA
            Some(Self::new(&trimmed[..7], is_dirty))
        } else {
            None
        }
    }
}

/// Context metadata for rendering the dynamic prompt.
pub struct PromptContext {
    pub user: String,
    pub hostname: String,
    pub cwd: String,
    pub home_dir: String,
    pub last_exit_code: i32,
    pub last_duration_ms: Option<u64>,
    pub git: Option<GitStatus>,
}

impl PromptContext {
    pub fn new() -> Self {
        Self {
            user: String::from("root"),
            hostname: String::from("novaos"),
            cwd: String::from("/home/firoze/projects/others/novaos"),
            home_dir: String::from("/home/firoze"),
            last_exit_code: 0,
            last_duration_ms: None,
            git: Some(GitStatus::new("master", false)),
        }
    }

    /// Abbreviate path with '~' if under home directory
    pub fn display_cwd(&self) -> String {
        if self.cwd == self.home_dir {
            String::from("~")
        } else if let Some(sub) = self.cwd.strip_prefix(&self.home_dir) {
            format!("~{}", sub)
        } else {
            self.cwd.clone()
        }
    }
}

/// Prompt formatting and ANSI generation.
pub struct PromptRenderer;

impl PromptRenderer {
    /// Render dynamic interactive prompt with git status and exit code
    pub fn render(ctx: &PromptContext) -> (String, usize) {
        let mut prompt = String::new();
        let mut visible_len = 0;

        // 1. User & Hostname: user@host
        let user_color = if ctx.user == "root" {
            "\x1b[1;31m" // Bold Red for root
        } else {
            "\x1b[1;32m" // Bold Green for normal user
        };
        prompt.push_str(user_color);
        prompt.push_str(&ctx.user);
        prompt.push_str("@");
        prompt.push_str(&ctx.hostname);
        prompt.push_str("\x1b[0m ");
        visible_len += ctx.user.len() + 1 + ctx.hostname.len() + 1;

        // 2. Current Working Directory: in ~/path
        let cwd_str = ctx.display_cwd();
        prompt.push_str("in \x1b[1;34m");
        prompt.push_str(&cwd_str);
        prompt.push_str("\x1b[0m");
        visible_len += 3 + cwd_str.len();

        // 3. Git Branch Status: on git:(branch*)
        if let Some(ref git) = ctx.git {
            prompt.push_str(" on \x1b[1;35m(");
            prompt.push_str(&git.branch);
            if git.is_dirty {
                prompt.push_str("\x1b[33m*");
            }
            prompt.push_str("\x1b[1;35m)\x1b[0m");
            visible_len += 4 + 1 + git.branch.len() + if git.is_dirty { 1 } else { 0 } + 1;
        }

        // 4. Execution Timer: [1.2s] if duration >= 1000ms
        if let Some(ms) = ctx.last_duration_ms {
            if ms >= 1000 {
                let secs = ms as f32 / 1000.0;
                let time_str = format!(" [{:.1}s]", secs);
                prompt.push_str("\x1b[33m");
                prompt.push_str(&time_str);
                prompt.push_str("\x1b[0m");
                visible_len += time_str.len();
            }
        }

        // 5. Prompt Symbol: ➜ (green if exit == 0, red ✗ if exit != 0)
        prompt.push_str("\r\n");
        if ctx.last_exit_code == 0 {
            prompt.push_str("\x1b[1;32m➜\x1b[0m ");
        } else {
            prompt.push_str("\x1b[1;31m✗\x1b[0m ");
        }
        visible_len += 2;

        (prompt, visible_len)
    }

    /// Render a simple compact one-line prompt
    pub fn render_compact(ctx: &PromptContext) -> (String, usize) {
        let cwd_str = ctx.display_cwd();
        let mut prompt = String::new();
        let mut visible_len = 0;

        prompt.push_str("\x1b[1;34m");
        prompt.push_str(&cwd_str);
        prompt.push_str("\x1b[0m");
        visible_len += cwd_str.len();

        if let Some(ref git) = ctx.git {
            let git_info = format!(" ({}{})", git.branch, if git.is_dirty { "*" } else { "" });
            prompt.push_str("\x1b[1;35m");
            prompt.push_str(&git_info);
            prompt.push_str("\x1b[0m");
            visible_len += git_info.len();
        }

        let symbol = if ctx.last_exit_code == 0 {
            " \x1b[1;32m$\x1b[0m "
        } else {
            " \x1b[1;31m$\x1b[0m "
        };
        prompt.push_str(symbol);
        visible_len += 3;

        (prompt, visible_len)
    }
}
