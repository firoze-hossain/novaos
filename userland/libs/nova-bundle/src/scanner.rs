//! userland/libs/nova-bundle/src/scanner.rs
//!
//! Application Bundle Discovery & Inventory Scanner.
//! Scans standard NovaOS application directories (/Applications, /System/Applications, ~/Applications),
//! parses manifests, validates bundles, and powers the Application Launcher & Dock.

#[cfg(not(test))]
use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[cfg(test)]
use std::{
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::layout::{APPLICATIONS_DIR, SYSTEM_APPS_DIR, USER_APPS_DIR};
use crate::AppBundle;

/// Registry and search index for discovered application bundles on NovaOS.
#[derive(Debug, Clone, Default)]
pub struct BundleScanner {
    /// Search paths where application bundles reside.
    pub search_paths: Vec<String>,
    /// Inventory of validated discovered application bundles.
    pub installed_bundles: Vec<AppBundle>,
}

impl BundleScanner {
    /// Creates a bundle scanner initialized with standard system and user application paths.
    pub fn new() -> Self {
        Self {
            search_paths: vec![
                SYSTEM_APPS_DIR.to_string(),
                APPLICATIONS_DIR.to_string(),
                USER_APPS_DIR.to_string(),
            ],
            installed_bundles: Vec::new(),
        }
    }

    /// Registers a discovered application bundle into the index.
    pub fn register_bundle(&mut self, bundle: AppBundle) {
        // Prevent duplicate registration by bundle_id (replace if exists)
        if let Some(pos) = self.installed_bundles.iter().position(|b| b.manifest.bundle_id == bundle.manifest.bundle_id) {
            self.installed_bundles[pos] = bundle;
        } else {
            self.installed_bundles.push(bundle);
        }
    }

    /// Removes an application bundle from the index (e.g. after uninstallation).
    pub fn unregister_bundle(&mut self, bundle_id: &str) -> bool {
        if let Some(pos) = self.installed_bundles.iter().position(|b| b.manifest.bundle_id == bundle_id) {
            self.installed_bundles.remove(pos);
            true
        } else {
            false
        }
    }

    /// Finds an installed bundle by exact bundle_id.
    pub fn find_by_id(&self, bundle_id: &str) -> Option<&AppBundle> {
        self.installed_bundles.iter().find(|b| b.manifest.bundle_id == bundle_id)
    }

    /// Finds all installed bundles matching a category.
    pub fn find_by_category(&self, category: &str) -> Vec<&AppBundle> {
        if category.eq_ignore_ascii_case("all") {
            return self.installed_bundles.iter().collect();
        }
        self.installed_bundles
            .iter()
            .filter(|b| b.manifest.categories.iter().any(|c| c.eq_ignore_ascii_case(category)))
            .collect()
    }

    /// Full-text search across application names, bundle IDs, descriptions, and categories.
    pub fn search(&self, query: &str) -> Vec<&AppBundle> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.installed_bundles.iter().collect();
        }

        self.installed_bundles
            .iter()
            .filter(|b| {
                b.manifest.name.to_lowercase().contains(&q)
                    || b.manifest.bundle_id.to_lowercase().contains(&q)
                    || b.manifest.description.to_lowercase().contains(&q)
                    || b.manifest.categories.iter().any(|c| c.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// Collects unique list of all categories present across installed bundles.
    pub fn all_categories(&self) -> Vec<String> {
        let mut list = vec!["All".to_string()];
        for b in &self.installed_bundles {
            for cat in &b.manifest.categories {
                if !list.iter().any(|existing| existing.eq_ignore_ascii_case(cat)) {
                    list.push(cat.clone());
                }
            }
        }
        list
    }
}
