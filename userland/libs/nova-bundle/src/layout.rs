//! userland/libs/nova-bundle/src/layout.rs
//!
//! macOS-style application bundle directory layout (`AppName.app`).
//! Defines directory structure, path resolution, component discovery, and structural verification.

#[cfg(not(test))]
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

#[cfg(test)]
use std::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use crate::manifest::BundleManifest;

pub const BUNDLE_EXTENSION: &str = ".app";
pub const MANIFEST_FILENAME: &str = "Manifest.json";
pub const BIN_DIRNAME: &str = "bin";
pub const ASSETS_DIRNAME: &str = "assets";
pub const LIB_DIRNAME: &str = "lib";

pub const SYSTEM_APPS_DIR: &str = "/System/Applications";
pub const APPLICATIONS_DIR: &str = "/Applications";
pub const USER_APPS_DIR: &str = "/users/root/Applications";

/// Structural errors in application bundle layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    InvalidExtension(String),
    MissingManifest,
    MissingBinaryDirectory,
    MissingExecutable(String),
    MissingAssetsDirectory,
    MissingIcon(String),
    EmptyBundle,
    CorruptedFileEntry(String),
}

/// An individual file or directory record inside an application bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleFileEntry {
    /// Relative path inside bundle (e.g., "Manifest.json", "bin/app", "lib/libengine.so").
    pub rel_path: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// True if entry is a directory.
    pub is_dir: bool,
    /// True if entry has executable permissions (ELF binary).
    pub is_executable: bool,
}

impl BundleFileEntry {
    pub fn new(rel_path: impl Into<String>, size_bytes: u64, is_dir: bool, is_executable: bool) -> Self {
        Self {
            rel_path: rel_path.into(),
            size_bytes,
            is_dir,
            is_executable,
        }
    }
}

/// Bundle directory layout and path resolution helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleLayout {
    /// Absolute or canonical root path to `.app` folder (e.g., "/Applications/Calculator.app").
    pub root_path: String,
    /// Extracted folder name (e.g., "Calculator.app").
    pub folder_name: String,
}

impl BundleLayout {
    /// Initializes a bundle layout descriptor from a root directory path.
    pub fn from_path(path: impl Into<String>) -> Result<Self, LayoutError> {
        let root = path.into();
        let trimmed = root.trim_end_matches('/');
        let folder_name = match trimmed.rfind('/') {
            Some(idx) => trimmed[idx + 1..].to_string(),
            None => trimmed.to_string(),
        };

        if !folder_name.ends_with(BUNDLE_EXTENSION) {
            return Err(LayoutError::InvalidExtension(folder_name));
        }

        Ok(Self {
            root_path: trimmed.to_string(),
            folder_name,
        })
    }

    /// Tests if a folder or filename ends with the standard `.app` bundle extension.
    pub fn is_bundle_path(path: &str) -> bool {
        path.trim_end_matches('/').ends_with(BUNDLE_EXTENSION)
    }

    /// Full path to `Manifest.json`.
    pub fn manifest_path(&self) -> String {
        format!("{}/{}", self.root_path, MANIFEST_FILENAME)
    }

    /// Full path to private binaries directory `bin/`.
    pub fn bin_dir(&self) -> String {
        format!("{}/{}", self.root_path, BIN_DIRNAME)
    }

    /// Full path to static assets directory `assets/`.
    pub fn assets_dir(&self) -> String {
        format!("{}/{}", self.root_path, ASSETS_DIRNAME)
    }

    /// Full path to private shared libraries directory `lib/`.
    pub fn lib_dir(&self) -> String {
        format!("{}/{}", self.root_path, LIB_DIRNAME)
    }

    /// Resolves absolute path to target executable declared in manifest.
    pub fn executable_path(&self, manifest_exec: &str) -> String {
        let clean = manifest_exec.trim_start_matches('/');
        format!("{}/{}", self.root_path, clean)
    }

    /// Resolves absolute path to main app icon declared in manifest.
    pub fn icon_path(&self, manifest_icon: &str) -> String {
        let clean = manifest_icon.trim_start_matches('/');
        format!("{}/{}", self.root_path, clean)
    }
}

/// Discovered contents and verification state of an application bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleStructure {
    /// All discovered files within the bundle.
    pub entries: Vec<BundleFileEntry>,
}

impl BundleStructure {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn with_entries(entries: Vec<BundleFileEntry>) -> Self {
        Self { entries }
    }

    pub fn add_entry(&mut self, entry: BundleFileEntry) {
        self.entries.push(entry);
    }

    /// Checks if a file exists relative to bundle root.
    pub fn has_file(&self, rel_path: &str) -> bool {
        let normalized = rel_path.trim_start_matches('/');
        self.entries.iter().any(|e| !e.is_dir && e.rel_path == normalized)
    }

    /// Checks if a directory exists relative to bundle root.
    pub fn has_dir(&self, rel_path: &str) -> bool {
        let normalized = rel_path.trim_start_matches('/').trim_end_matches('/');
        self.entries.iter().any(|e| e.is_dir && e.rel_path.trim_end_matches('/') == normalized)
    }

    /// Discovers private shared libraries in `lib/`.
    pub fn private_shared_libraries(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| !e.is_dir && e.rel_path.starts_with("lib/") && (e.rel_path.ends_with(".so") || e.rel_path.ends_with(".rlib")))
            .map(|e| e.rel_path.clone())
            .collect()
    }

    /// Total bundle uncompressed size in bytes.
    pub fn total_size_bytes(&self) -> u64 {
        self.entries.iter().map(|e| e.size_bytes).sum()
    }

    /// Verifies bundle internal consistency against declared manifest.
    pub fn verify_integrity(&self, manifest: &BundleManifest) -> Result<(), LayoutError> {
        if self.entries.is_empty() {
            return Err(LayoutError::EmptyBundle);
        }

        // 1. Verify Manifest.json exists
        if !self.has_file(MANIFEST_FILENAME) {
            return Err(LayoutError::MissingManifest);
        }

        // 2. Verify bin/ directory exists
        if !self.has_dir(BIN_DIRNAME) && !self.entries.iter().any(|e| e.rel_path.starts_with("bin/")) {
            return Err(LayoutError::MissingBinaryDirectory);
        }

        // 3. Verify main executable exists and is marked executable
        let exec_rel = manifest.executable.trim_start_matches('/');
        let exec_entry = self.entries.iter().find(|e| e.rel_path == exec_rel);
        match exec_entry {
            Some(e) if e.is_executable => {}
            Some(_) => return Err(LayoutError::MissingExecutable(format!("{} is not executable", exec_rel))),
            None => return Err(LayoutError::MissingExecutable(format!("{} not found in bundle", exec_rel))),
        }

        // 4. Verify assets/ exists
        if !self.has_dir(ASSETS_DIRNAME) && !self.entries.iter().any(|e| e.rel_path.starts_with("assets/")) {
            return Err(LayoutError::MissingAssetsDirectory);
        }

        // 5. Verify icon exists
        let icon_rel = manifest.icon.trim_start_matches('/');
        if !self.has_file(icon_rel) {
            return Err(LayoutError::MissingIcon(format!("{} not found in bundle", icon_rel)));
        }

        Ok(())
    }
}

impl Default for BundleStructure {
    fn default() -> Self {
        Self::new()
    }
}
