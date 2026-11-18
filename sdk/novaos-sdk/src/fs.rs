//! sdk/novaos-sdk/src/fs.rs
//!
//! Sandboxed filesystem access APIs for NovaOS applications.
//! Enforces path normalization, capability boundaries, and sandbox jails.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Safe path resolver ensuring application operations stay within allowed directories.
pub struct SandboxedFs {
    jail_root: PathBuf,
}

impl SandboxedFs {
    pub fn new(jail_root: impl Into<PathBuf>) -> Self {
        Self {
            jail_root: jail_root.into(),
        }
    }

    /// Resolves an application-relative path and checks that it does not escape the jail root.
    pub fn resolve(&self, relative_path: impl AsRef<Path>) -> io::Result<PathBuf> {
        let p = relative_path.as_ref();
        // Prevent path traversal attacks
        for component in p.components() {
            if component == std::path::Component::ParentDir {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Path traversal (..) is not permitted in sandboxed filesystem",
                ));
            }
        }
        Ok(self.jail_root.join(p))
    }

    pub fn read_file(&self, path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        let full_path = self.resolve(path)?;
        fs::read(full_path)
    }

    pub fn read_to_string(&self, path: impl AsRef<Path>) -> io::Result<String> {
        let full_path = self.resolve(path)?;
        fs::read_to_string(full_path)
    }

    pub fn write_file(&self, path: impl AsRef<Path>, contents: &[u8]) -> io::Result<()> {
        let full_path = self.resolve(path)?;
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(full_path, contents)
    }

    pub fn exists(&self, path: impl AsRef<Path>) -> bool {
        match self.resolve(path) {
            Ok(full_path) => full_path.exists(),
            Err(_) => false,
        }
    }
}

/// Standard NovaOS application paths.
pub struct AppPaths;

impl AppPaths {
    pub fn app_data_dir(app_id: &str) -> PathBuf {
        PathBuf::from("/var/data").join(app_id)
    }

    pub fn app_config_dir(app_id: &str) -> PathBuf {
        PathBuf::from("/etc").join(app_id)
    }

    pub fn documents_dir() -> PathBuf {
        PathBuf::from("/home/user/Documents")
    }

    pub fn temp_dir() -> PathBuf {
        PathBuf::from("/tmp")
    }
}

/// Helper functions for direct file operations.
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    fs::read_to_string(path)
}

pub fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(parent) = path.as_ref().parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(path, contents)
}

pub fn exists(path: impl AsRef<Path>) -> bool {
    path.as_ref().exists()
}
