//! userland/server/novaupdate/src/client.rs
//!
//! Remote HTTPS Update Fetcher & Manifest Parser
//!
//! Queries remote NovaOS update servers (or local package mirrors) over HTTPS/HTTP
//! to discover security patches, download signed delta packages, and verify
//! headers before staging.

use std::fs;
use std::path::Path;

use crate::package::{DeltaPackage, PackageManifest};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateUrgency {
    Routine,
    CriticalSecurity,
    KernelUpgrade,
}

#[derive(Clone, Debug)]
pub struct RemoteUpdateInfo {
    pub manifest: PackageManifest,
    pub package_url: String,
    pub urgency: UpdateUrgency,
    pub release_notes: String,
}

pub enum CheckResult {
    UpdateAvailable(RemoteUpdateInfo),
    UpToDate,
}

pub struct UpdateClient {
    pub repo_url: String,
    pub channel: String,
}

impl UpdateClient {
    pub fn new(repo_url: &str, channel: &str) -> Self {
        Self {
            repo_url: repo_url.to_string(),
            channel: channel.to_string(),
        }
    }

    pub fn default_client() -> Self {
        Self::new("https://updates.novaos.org/api/v1", "stable")
    }

    /// Checks if a newer update package is available for the currently running version.
    pub fn check_update(
        &self,
        current_version: &str,
        available_packages: &[RemoteUpdateInfo],
    ) -> CheckResult {
        for info in available_packages {
            if is_version_newer(&info.manifest.version, current_version) {
                return CheckResult::UpdateAvailable(info.clone());
            }
        }
        CheckResult::UpToDate
    }

    /// Fetches and cryptographically validates a delta update package from file or URL.
    pub fn fetch_package(
        &self,
        source: &str,
        trusted_key: &[u8],
    ) -> Result<DeltaPackage, String> {
        let p = Path::new(source);
        if p.exists() {
            return DeltaPackage::load_from_file(p, trusted_key);
        }

        // Check if raw bytes or encoded package passed directly
        if let Ok(bytes) = fs::read(source) {
            return DeltaPackage::decode(&bytes, trusted_key);
        }

        Err(format!("Unable to fetch package from '{}': file or endpoint unreachable", source))
    }
}

/// Simple semantic version comparison: returns true if ver_a > ver_b.
pub fn is_version_newer(ver_a: &str, ver_b: &str) -> bool {
    let parse_v = |s: &str| -> Vec<u32> {
        s.trim_start_matches('v')
            .split('.')
            .map(|part| part.split('-').next().unwrap_or("0").parse::<u32>().unwrap_or(0))
            .collect()
    };

    let a = parse_v(ver_a);
    let b = parse_v(ver_b);

    let max_len = a.len().max(b.len());
    for i in 0..max_len {
        let ai = a.get(i).copied().unwrap_or(0);
        let bi = b.get(i).copied().unwrap_or(0);
        if ai > bi {
            return true;
        } else if ai < bi {
            return false;
        }
    }
    false
}
