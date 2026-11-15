//! userland/desktop/apps/novash/src/bookmarks.rs
//!
//! Directory Bookmarks and Directory Stack (`pushd`/`popd`/`dirs`) for NovaShell.
//! Inspired by z/autojump and zsh directory navigation.

#[cfg(not(test))]
use alloc::{format, string::String, string::ToString, vec::Vec};
#[cfg(test)]
use std::{format, string::String, string::ToString, vec::Vec};

/// A named directory bookmark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub name: String,
    pub path: String,
}

/// Directory bookmark and navigation manager.
pub struct BookmarkManager {
    pub bookmarks: Vec<Bookmark>,
    pub dir_stack: Vec<String>,
}

impl BookmarkManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            bookmarks: Vec::new(),
            dir_stack: Vec::new(),
        };

        // Standard default bookmarks
        mgr.add("root", "/");
        mgr.add("home", "/home");
        mgr.add("bin", "/bin");
        mgr.add("docs", "/docs");
        mgr
    }

    /// Add or update a named bookmark
    pub fn add(&mut self, name: &str, path: &str) {
        if let Some(existing) = self.bookmarks.iter_mut().find(|b| b.name == name) {
            existing.path = path.to_string();
        } else {
            self.bookmarks.push(Bookmark {
                name: name.to_string(),
                path: path.to_string(),
            });
        }
    }

    /// Remove a bookmark by name
    pub fn remove(&mut self, name: &str) -> bool {
        if let Some(pos) = self.bookmarks.iter().position(|b| b.name == name) {
            self.bookmarks.remove(pos);
            true
        } else {
            false
        }
    }

    /// Retrieve path for a bookmark name
    pub fn get(&self, name: &str) -> Option<&str> {
        self.bookmarks
            .iter()
            .find(|b| b.name == name)
            .map(|b| b.path.as_str())
    }

    /// Resolves an argument that may be `@bookmark` or a bare bookmark name
    pub fn resolve(&self, target: &str) -> Option<String> {
        if let Some(bare) = target.strip_prefix('@') {
            self.get(bare).map(|s| s.to_string())
        } else {
            self.get(target).map(|s| s.to_string())
        }
    }

    /// Get all bookmark names for completion
    pub fn names(&self) -> Vec<String> {
        self.bookmarks.iter().map(|b| b.name.clone()).collect()
    }

    /// Push current directory onto stack and change to target directory
    pub fn pushd(&mut self, current_cwd: &str, target_dir: &str) -> Result<String, &'static str> {
        self.dir_stack.push(current_cwd.to_string());
        Ok(target_dir.to_string())
    }

    /// Pop top directory from stack and return it as the new directory
    pub fn popd(&mut self) -> Result<String, &'static str> {
        if let Some(prev) = self.dir_stack.pop() {
            Ok(prev)
        } else {
            Err("Directory stack is empty")
        }
    }

    /// Format directory stack (like `dirs -v`)
    pub fn list_dirs(&self, current_cwd: &str) -> Vec<String> {
        let mut list = Vec::new();
        list.push(format!("0  {}", current_cwd));
        for (idx, d) in self.dir_stack.iter().rev().enumerate() {
            list.push(format!("{}  {}", idx + 1, d));
        }
        list
    }
}
