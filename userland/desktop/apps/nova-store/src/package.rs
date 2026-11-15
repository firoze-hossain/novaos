//! userland/desktop/apps/nova-store/src/package.rs
//!
//! Application Package Models, Categories, Installation State, and Metadata.

#[cfg(not(test))]
use alloc::{
    string::String,
    vec::Vec,
};

#[cfg(test)]
use std::{
    string::String,
    vec::Vec,
};

use crate::sandbox::SandboxPermissions;

/// Store navigation categories synthesizing Ubuntu Software Center and macOS App Store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppCategory {
    Featured,
    Development,
    Productivity,
    Utilities,
    Internet,
    Installed,
    Updates,
}

impl AppCategory {
    pub const fn id(&self) -> &'static str {
        match self {
            Self::Featured => "featured",
            Self::Development => "development",
            Self::Productivity => "productivity",
            Self::Utilities => "utilities",
            Self::Internet => "internet",
            Self::Installed => "installed",
            Self::Updates => "updates",
        }
    }

    pub const fn title(&self) -> &'static str {
        match self {
            Self::Featured => "Discover & Featured",
            Self::Development => "Developer Tools",
            Self::Productivity => "Productivity & Office",
            Self::Utilities => "System Utilities",
            Self::Internet => "Internet & Networking",
            Self::Installed => "Installed Applications",
            Self::Updates => "Available Updates",
        }
    }

    pub const fn icon(&self) -> &'static str {
        match self {
            Self::Featured => "[*]",
            Self::Development => "[DEV]",
            Self::Productivity => "[DOC]",
            Self::Utilities => "[UTL]",
            Self::Internet => "[NET]",
            Self::Installed => "[PKG]",
            Self::Updates => "[UPD]",
        }
    }

    pub const fn all() -> &'static [AppCategory] {
        &[
            Self::Featured,
            Self::Development,
            Self::Productivity,
            Self::Utilities,
            Self::Internet,
            Self::Installed,
            Self::Updates,
        ]
    }
}

/// Installation lifecycle state.
#[derive(Clone, Debug, PartialEq)]
pub enum InstallStatus {
    NotInstalled,
    Installing { progress_pct: u8 },
    Installed { version: String },
    Updatable { installed_version: String, new_version: String },
    Removing,
}

/// Visual screenshot card preview.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenshotPreview {
    pub title: String,
    pub caption: String,
    pub accent_rgb: (u8, u8, u8),
}

/// Rich application package record.
#[derive(Clone, Debug, PartialEq)]
pub struct AppPackage {
    pub id: String,
    pub name: String,
    pub version: String,
    pub category: AppCategory,
    pub developer: String,
    pub license: String,
    pub summary: String,
    pub description: String,
    pub changelog: String,
    pub icon_symbol: &'static str,
    pub download_size_bytes: u64,
    pub installed_size_bytes: u64,
    pub rating: f32,
    pub rating_count: u32,
    pub dependencies: Vec<String>,
    pub permissions: SandboxPermissions,
    pub sha256_hash: String,
    pub signature_ed25519: String,
    pub status: InstallStatus,
    pub screenshots: Vec<ScreenshotPreview>,
}

impl AppPackage {
    pub fn is_installed(&self) -> bool {
        matches!(self.status, InstallStatus::Installed { .. } | InstallStatus::Updatable { .. })
    }

    pub fn is_updatable(&self) -> bool {
        matches!(self.status, InstallStatus::Updatable { .. })
    }

    pub fn formatted_size(&self) -> String {
        let mb = self.download_size_bytes as f64 / (1024.0 * 1024.0);
        if mb >= 1.0 {
            #[cfg(test)]
            return std::format!("{:.1} MB", mb);
            #[cfg(not(test))]
            return alloc::format!("{} MB", self.download_size_bytes / (1024 * 1024));
        }
        let kb = self.download_size_bytes / 1024;
        #[cfg(test)]
        return std::format!("{} KB", kb);
        #[cfg(not(test))]
        return alloc::format!("{} KB", kb);
    }
}
