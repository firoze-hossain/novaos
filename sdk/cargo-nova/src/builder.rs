//! sdk/cargo-nova/src/builder.rs
//!
//! Invokes cargo build for NovaOS application projects.

use std::path::{Path, PathBuf};
use std::process::Command;

pub struct BuildOptions {
    pub release: bool,
    pub target: Option<String>,
    pub quiet: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            release: false,
            target: None,
            quiet: false,
        }
    }
}

pub struct AppBuilder;

impl AppBuilder {
    pub fn build(project_dir: &Path, opts: &BuildOptions) -> Result<PathBuf, String> {
        let manifest_path = project_dir.join("Cargo.toml");
        if !manifest_path.exists() {
            return Err(format!("Cargo.toml not found in {}", project_dir.display()));
        }

        let mut cmd = Command::new("cargo");
        cmd.arg("build");
        cmd.arg("--manifest-path").arg(&manifest_path);

        if opts.release {
            cmd.arg("--release");
        }
        if let Some(ref t) = opts.target {
            cmd.arg("--target").arg(t);
        }
        if opts.quiet {
            cmd.arg("--quiet");
        }

        let status = cmd.status().map_err(|e| format!("Failed to spawn cargo build: {}", e))?;
        if !status.success() {
            return Err("cargo build exited with non-zero status".to_string());
        }

        // Determine output binary path
        let target_subdir = if opts.release { "release" } else { "debug" };
        let target_dir = project_dir.join("target").join(target_subdir);

        // Read package name from Cargo.toml
        let cargo_text = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("Failed to read Cargo.toml: {}", e))?;
        let pkg_name = Self::extract_package_name(&cargo_text)
            .ok_or_else(|| "Could not determine package name from Cargo.toml".to_string())?;

        let bin_path = target_dir.join(&pkg_name);
        if !bin_path.exists() {
            return Err(format!("Compiled binary not found at {}", bin_path.display()));
        }

        Ok(bin_path)
    }

    pub fn extract_package_name(cargo_toml_content: &str) -> Option<String> {
        for line in cargo_toml_content.lines() {
            let line = line.trim();
            if line.starts_with("name") && line.contains('=') {
                let parts: Vec<&str> = line.splitn(2, '=').collect();
                if parts.len() == 2 {
                    let val = parts[1].trim().trim_matches('"').trim_matches('\'').trim();
                    return Some(val.to_string());
                }
            }
        }
        None
    }
}
