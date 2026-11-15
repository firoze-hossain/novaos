//! userland/libs/nova-bundle/src/builder.rs
//!
//! Programmatic Builder API for creating and packaging self-contained `.app` bundles.

#[cfg(not(test))]
use alloc::{
    format,
    string::String,
    vec::Vec,
};

#[cfg(test)]
use std::{
    format,
    string::String,
    vec::Vec,
};

use crate::layout::{BundleFileEntry, BundleLayout, BundleStructure, MANIFEST_FILENAME};
use crate::manifest::{BundleManifest, ManifestError, SandboxPermissions};
use crate::AppBundle;

/// Fluent builder for constructing complete NovaOS application bundles.
#[derive(Debug, Clone)]
pub struct BundleBuilder {
    manifest: BundleManifest,
    entries: Vec<BundleFileEntry>,
}

impl BundleBuilder {
    /// Initializes a new application bundle builder with primary metadata.
    pub fn new(
        bundle_id: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        let bid = bundle_id.into();
        let n = name.into();
        let v = version.into();
        let exec_name = n.to_lowercase().replace(' ', "-");
        let default_exec = format!("bin/{}", exec_name);

        let manifest = BundleManifest::new(bid, n, v, default_exec);

        Self {
            manifest,
            entries: Vec::new(),
        }
    }

    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.manifest.description = desc.into();
        self
    }

    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.manifest.author = author.into();
        self
    }

    pub fn categories(mut self, cats: Vec<String>) -> Self {
        self.manifest.categories = cats;
        self
    }

    pub fn permissions(mut self, sandbox: SandboxPermissions) -> Self {
        self.manifest.sandbox = sandbox;
        self
    }

    pub fn env_var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.manifest.env_vars.push((key.into(), value.into()));
        self
    }

    pub fn mime_type(mut self, mime: impl Into<String>) -> Self {
        self.manifest.mime_types.push(mime.into());
        self
    }

    /// Adds the primary 64-bit ELF binary into `bin/`.
    pub fn set_primary_executable(mut self, binary_name: &str, size_bytes: u64) -> Self {
        let clean = binary_name.trim_start_matches('/');
        let rel_path = format!("bin/{}", clean);
        self.manifest.executable = rel_path.clone();

        self.entries.retain(|e| !e.rel_path.starts_with("bin/"));
        self.entries.push(BundleFileEntry::new("bin", 0, true, false));
        self.entries.push(BundleFileEntry::new(rel_path, size_bytes, false, true));
        self
    }

    /// Adds the main application icon into `assets/`.
    pub fn set_main_icon(mut self, icon_name: &str, size_bytes: u64) -> Self {
        let clean = icon_name.trim_start_matches('/');
        let rel_path = format!("assets/{}", clean);
        self.manifest.icon = rel_path.clone();

        if !self.entries.iter().any(|e| e.rel_path == "assets") {
            self.entries.push(BundleFileEntry::new("assets", 0, true, false));
        }
        self.entries.push(BundleFileEntry::new(rel_path, size_bytes, false, false));
        self
    }

    /// Adds a private shared dynamic library into `lib/` (resolving dependency hell).
    pub fn add_shared_library(mut self, lib_name: &str, size_bytes: u64) -> Self {
        let clean = lib_name.trim_start_matches('/');
        let rel_path = format!("lib/{}", clean);

        if !self.entries.iter().any(|e| e.rel_path == "lib") {
            self.entries.push(BundleFileEntry::new("lib", 0, true, false));
        }
        self.entries.push(BundleFileEntry::new(rel_path, size_bytes, false, false));
        self
    }

    /// Adds an arbitrary resource asset into `assets/`.
    pub fn add_asset(mut self, rel_asset_path: &str, size_bytes: u64) -> Self {
        let clean = rel_asset_path.trim_start_matches('/');
        let rel_path = format!("assets/{}", clean);

        if !self.entries.iter().any(|e| e.rel_path == "assets") {
            self.entries.push(BundleFileEntry::new("assets", 0, true, false));
        }
        self.entries.push(BundleFileEntry::new(rel_path, size_bytes, false, false));
        self
    }

    /// Builds and packages the finalized `AppBundle`.
    pub fn build(mut self, destination_path: &str) -> Result<AppBundle, ManifestError> {
        self.manifest.validate()?;

        // Add Manifest.json entry
        let manifest_json = self.manifest.to_json();
        self.entries.insert(
            0,
            BundleFileEntry::new(MANIFEST_FILENAME, manifest_json.len() as u64, false, false),
        );

        let layout = BundleLayout::from_path(destination_path)
            .map_err(|e| ManifestError::SemanticError(format!("{:?}", e)))?;

        let structure = BundleStructure::with_entries(self.entries);

        let bundle = AppBundle {
            layout,
            manifest: self.manifest,
            structure,
        };

        bundle
            .verify()
            .map_err(|e| ManifestError::SemanticError(format!("{:?}", e)))?;

        Ok(bundle)
    }
}
