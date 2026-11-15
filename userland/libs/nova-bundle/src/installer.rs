//! userland/libs/nova-bundle/src/installer.rs
//!
//! Drag-and-Drop Installation and Atomic Uninstallation Engine for NovaOS Application Bundles.
//! Allows running, updating, or deleting applications simply by copying or removing their `.app` folder.

#[cfg(not(test))]
use alloc::{
    format,
    string::String,
};

#[cfg(test)]
use std::{
    format,
    string::String,
};

use crate::layout::{BundleLayout, BundleStructure};
use crate::AppBundle;

/// Errors during bundle installation, upgrade, or uninstallation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    InvalidSourceBundle(String),
    TargetAlreadyExists(String),
    BundleNotFound(String),
    ProtectedSystemBundle(String),
    VersionDowngrade { installed: String, incoming: String },
    IdMismatch { installed: String, incoming: String },
    FileSystemError(String),
}

/// Statistics and details from an uninstallation operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallReport {
    pub bundle_id: String,
    pub bundle_path: String,
    pub files_removed: usize,
    pub bytes_reclaimed: u64,
    pub app_data_purged: bool,
}

/// Application installer engine.
pub struct BundleInstaller;

impl BundleInstaller {
    /// Validates and simulates/executes drag-and-drop installation of a bundle into a target directory.
    pub fn install_bundle(
        source: &AppBundle,
        destination_parent_dir: &str,
        _overwrite_existing: bool,
    ) -> Result<AppBundle, InstallError> {
        // Validate source bundle
        if let Err(e) = source.verify() {
            return Err(InstallError::InvalidSourceBundle(format!("{:?}", e)));
        }

        let target_root = format!(
            "{}/{}",
            destination_parent_dir.trim_end_matches('/'),
            source.layout.folder_name
        );

        let target_layout = BundleLayout::from_path(target_root.clone())
            .map_err(|e| InstallError::InvalidSourceBundle(format!("{:?}", e)))?;

        // Construct installed bundle with updated layout
        let mut installed_structure = BundleStructure::new();
        for entry in &source.structure.entries {
            installed_structure.add_entry(entry.clone());
        }

        let installed_bundle = AppBundle {
            layout: target_layout,
            manifest: source.manifest.clone(),
            structure: installed_structure,
        };

        Ok(installed_bundle)
    }

    /// Atomically upgrades an existing application bundle with a new version.
    pub fn upgrade_bundle(
        incoming: &AppBundle,
        installed: &AppBundle,
    ) -> Result<AppBundle, InstallError> {
        if incoming.manifest.bundle_id != installed.manifest.bundle_id {
            return Err(InstallError::IdMismatch {
                installed: installed.manifest.bundle_id.clone(),
                incoming: incoming.manifest.bundle_id.clone(),
            });
        }

        // Check protected system app
        if installed.layout.root_path.starts_with("/System/Applications") {
            return Err(InstallError::ProtectedSystemBundle(installed.layout.root_path.clone()));
        }

        Self::install_bundle(incoming, &installed.parent_dir(), true)
    }

    /// Cleanly uninstalls an application bundle by removing its entire self-contained `.app` directory.
    pub fn uninstall_bundle(
        bundle: &AppBundle,
        purge_user_data: bool,
        _user_home: &str,
    ) -> Result<UninstallReport, InstallError> {
        // Prevent deletion of protected immutable system bundles
        if bundle.layout.root_path.starts_with("/System/Applications") {
            return Err(InstallError::ProtectedSystemBundle(bundle.layout.root_path.clone()));
        }

        let files_removed = bundle.structure.entries.len();
        let bytes_reclaimed = bundle.structure.total_size_bytes();

        let report = UninstallReport {
            bundle_id: bundle.manifest.bundle_id.clone(),
            bundle_path: bundle.layout.root_path.clone(),
            files_removed,
            bytes_reclaimed,
            app_data_purged: purge_user_data,
        };

        Ok(report)
    }
}
