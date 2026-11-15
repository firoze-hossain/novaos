//! userland/desktop/apps/nova-store/src/lib.rs
//!
//! # NovaOS Graphical Software Store & Package Manager (nova-store)
//!
//! Synthesizes Ubuntu Software Center and macOS App Store into a unified,
//! secure, and discoverable software ecosystem.
//!
//! Features:
//! - Categorized browsing: Discover/Featured, Development, Productivity, Utilities, Internet, Installed, Updates.
//! - Real-time full-text search across app titles, summaries, descriptions, and developers.
//! - App details inspector with screenshot gallery, changelog, and license details.
//! - One-click lifecycle actions: GET/INSTALL, UPDATE, OPEN, REMOVE.
//! - Pure #![no_std] Ed25519 & SHA-256 cryptographic signature verification.
//! - Sandbox permission declarations (Network, Filesystem, Audio, Camera, GPU, Background).
//! - Recursive dependency tree resolution and reverse dependency uninstallation protection.
//! - High-performance acrylic UI rendering into 32-bit BufferSurface.

#![cfg_attr(not(test), no_std)]

#[cfg(not(test))]
extern crate alloc;

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

pub mod crypto;
pub mod deps;
pub mod package;
pub mod repo;
pub mod sandbox;
pub mod view;

use nova_ui::prelude::{BufferSurface, Point};
use package::{AppCategory, AppPackage};
use repo::PackageRepository;
use view::{StoreHitTarget, StoreView};

/// Top-level application controller for Nova Store.
pub struct NovaStoreApp {
    pub active_category: AppCategory,
    pub search_query: String,
    pub selected_app_id: Option<String>,
    pub repo: PackageRepository,
    pub view: StoreView,
}

impl Default for NovaStoreApp {
    fn default() -> Self {
        Self::new()
    }
}

impl NovaStoreApp {
    pub fn new() -> Self {
        Self {
            active_category: AppCategory::Featured,
            search_query: String::new(),
            selected_app_id: None,
            repo: PackageRepository::default(),
            view: StoreView::default(),
        }
    }

    /// Selects an active category and clears search/detail state.
    pub fn select_category(&mut self, cat: AppCategory) {
        self.active_category = cat;
        self.search_query.clear();
        self.selected_app_id = None;
    }

    /// Updates search query filter.
    pub fn set_search_query(&mut self, query: &str) {
        self.search_query = query.to_string();
        self.selected_app_id = None;
    }

    /// Clears search filter.
    pub fn clear_search(&mut self) {
        self.search_query.clear();
    }

    /// Opens the detailed inspector view for an application.
    pub fn select_app(&mut self, app_id: &str) {
        self.selected_app_id = Some(app_id.to_string());
    }

    /// Returns to catalog list view from app details.
    pub fn back_to_catalog(&mut self) {
        self.selected_app_id = None;
    }

    /// Installs a package by ID along with its dependencies.
    pub fn install(&mut self, app_id: &str) -> Result<Vec<String>, &'static str> {
        self.repo.install_package(app_id)
    }

    /// Updates an installed package to its latest version.
    pub fn update(&mut self, app_id: &str) -> Result<(), &'static str> {
        self.repo.update_package(app_id)
    }

    /// Removes an installed package safely.
    pub fn remove(&mut self, app_id: &str) -> Result<(), String> {
        self.repo.remove_package(app_id)
    }

    /// Handles pointer click events.
    pub fn handle_click(&mut self, pt: Point) -> bool {
        let current_packages: Vec<&AppPackage> = if !self.search_query.trim().is_empty() {
            self.repo.search(&self.search_query)
        } else {
            self.repo.get_by_category(self.active_category)
        };

        let selected = self
            .selected_app_id
            .as_ref()
            .and_then(|id| self.repo.packages.get(id));

        if let Some(target) = self.view.hit_test(
            pt,
            &self.search_query,
            &current_packages,
            selected,
        ) {
            match target {
                StoreHitTarget::Category(cat) => {
                    self.select_category(cat);
                    true
                }
                StoreHitTarget::SearchBox => true,
                StoreHitTarget::ClearSearch => {
                    self.clear_search();
                    true
                }
                StoreHitTarget::SelectApp(id) => {
                    self.select_app(&id);
                    true
                }
                StoreHitTarget::BackToCatalog => {
                    self.back_to_catalog();
                    true
                }
                StoreHitTarget::InstallPackage(id) => {
                    self.install(&id).is_ok()
                }
                StoreHitTarget::UpdatePackage(id) => {
                    self.update(&id).is_ok()
                }
                StoreHitTarget::RemovePackage(id) => {
                    self.remove(&id).is_ok()
                }
            }
        } else {
            false
        }
    }

    /// Renders the complete store interface.
    pub fn render(&self, surface: &mut BufferSurface) {
        let current_packages: Vec<&AppPackage> = if !self.search_query.trim().is_empty() {
            self.repo.search(&self.search_query)
        } else {
            self.repo.get_by_category(self.active_category)
        };

        let selected = self
            .selected_app_id
            .as_ref()
            .and_then(|id| self.repo.packages.get(id));

        let installed_count = self.repo.get_by_category(AppCategory::Installed).len();
        let updates_count = self.repo.get_by_category(AppCategory::Updates).len();

        self.view.render(
            surface,
            self.active_category,
            &self.search_query,
            &current_packages,
            selected,
            installed_count,
            updates_count,
        );
    }
}

// ============================================================================
// Comprehensive Self-Test Suite
// ============================================================================

/// Runs the 10-point self-test suite for nova-store.
pub fn run_nova_store_selftest() -> bool {
    let mut app = NovaStoreApp::new();

    // 1. Verify Category Registration and Repository Initial State
    let categories = AppCategory::all();
    if categories.len() != 7 {
        return false;
    }
    if app.repo.packages.len() < 8 {
        return false;
    }

    // 2. Verify Cryptographic Signature Verification
    let code_pkg = app.repo.packages.get("org.novaos.code").unwrap();
    let payload = format!("{}-{}-{}", code_pkg.id, code_pkg.version, code_pkg.summary);
    let valid_hash = crypto::sha256(payload.as_bytes());
    // Positive verification
    if !crypto::verify_package_signature(&code_pkg.id, &code_pkg.version, &valid_hash, &code_pkg.signature_ed25519) {
        return false;
    }
    // Negative verification: altered payload
    let bad_hash = crypto::sha256(b"tampered-payload-binary");
    if crypto::verify_package_signature(&code_pkg.id, &code_pkg.version, &bad_hash, &code_pkg.signature_ed25519) {
        return false;
    }

    // 3. Verify Sandbox Permissions Declaration
    let sysmon_pkg = app.repo.packages.get("org.novaos.sysmon").unwrap();
    if sysmon_pkg.permissions.is_sandboxed() {
        return false; // system utility has system_root
    }
    let perms = code_pkg.permissions.list_declared_permissions();
    if perms.is_empty() || !code_pkg.permissions.is_sandboxed() {
        return false;
    }

    // 4. Verify Dependency Tree Resolution
    let install_order = deps::DependencyResolver::resolve_install_order(
        "org.novaos.code",
        &app.repo.packages,
    );
    if let Ok(order) = install_order {
        // org.novaos.libgit MUST be installed before org.novaos.code
        if order.len() != 2 || order[0] != "org.novaos.libgit" || order[1] != "org.novaos.code" {
            return false;
        }
    } else {
        return false;
    }

    // 5. Verify One-Click Package Installation with Dependencies
    let install_res = app.install("org.novaos.code");
    if let Ok(installed_names) = install_res {
        if installed_names.len() != 2 {
            return false;
        }
    } else {
        return false;
    }
    // Check both packages are now marked installed
    if !app.repo.packages.get("org.novaos.code").unwrap().is_installed() {
        return false;
    }
    if !app.repo.packages.get("org.novaos.libgit").unwrap().is_installed() {
        return false;
    }

    // 6. Verify Reverse Dependency Protection on Removal
    // Removing libgit should fail because code depends on it
    let remove_libgit_res = app.remove("org.novaos.libgit");
    if remove_libgit_res.is_ok() {
        return false; // MUST fail
    }

    // 7. Verify One-Click Package Update Workflow
    let updates = app.repo.get_by_category(AppCategory::Updates);
    if updates.is_empty() || updates[0].id != "org.novaos.sysmon" {
        return false;
    }
    // Perform update
    if app.update("org.novaos.sysmon").is_err() {
        return false;
    }
    if app.repo.packages.get("org.novaos.sysmon").unwrap().is_updatable() {
        return false; // No longer updatable once updated to latest
    }

    // 8. Verify One-Click Package Uninstallation Workflow
    // Remove code first
    if app.remove("org.novaos.code").is_err() {
        return false;
    }
    if app.repo.packages.get("org.novaos.code").unwrap().is_installed() {
        return false;
    }
    // Now libgit can be safely removed
    if app.remove("org.novaos.libgit").is_err() {
        return false;
    }

    // 9. Verify Real-Time Search Engine
    let search_res = app.repo.search("editor");
    if search_res.is_empty() || search_res[0].id != "org.novaos.code" {
        return false;
    }
    let search_calc = app.repo.search("calculator");
    if search_calc.is_empty() || search_calc[0].id != "org.novaos.calc" {
        return false;
    }

    // 10. Verify BufferSurface Rasterization and Hit-Testing
    let mut surface = BufferSurface::new(1024, 680);
    app.render(&mut surface);

    let non_zero_pixels = surface
        .pixels
        .iter()
        .filter(|&&p| p != 0)
        .count();
    if non_zero_pixels < 12000 {
        return false;
    }

    // Hit test navigation to Productivity category
    let hit = app.handle_click(Point::new(80, 196));
    if !hit || app.active_category != AppCategory::Productivity {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nova_store_selftest() {
        assert!(run_nova_store_selftest(), "All nova-store self-tests pass");
    }

    #[test]
    fn test_category_properties() {
        for cat in AppCategory::all() {
            assert!(!cat.id().is_empty());
            assert!(!cat.title().is_empty());
            assert!(!cat.icon().is_empty());
        }
    }

    #[test]
    fn test_ed25519_crypto_verification() {
        let data = b"payload test data for sha256";
        let hash = crypto::sha256(data);
        let sig = crypto::sign_package("org.test.pkg", "1.0.0", &hash);
        assert!(crypto::verify_package_signature("org.test.pkg", "1.0.0", &hash, &sig));
        assert!(!crypto::verify_package_signature("org.test.pkg", "1.0.1", &hash, &sig));
    }

    #[test]
    fn test_sandbox_permissions_listing() {
        let perms = sandbox::SandboxPermissions::standard_app();
        assert!(perms.is_sandboxed());
        let list = perms.list_declared_permissions();
        assert!(list.iter().any(|p| p.contains("Network")));
        assert!(list.iter().any(|p| p.contains("Filesystem")));
    }

    #[test]
    fn test_search_filtering() {
        let repo = PackageRepository::default();
        let res = repo.search("matrix");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].id, "org.novaos.chat");
    }

    #[test]
    fn test_detail_view_navigation() {
        let mut app = NovaStoreApp::new();
        app.select_app("org.novaos.code");
        assert_eq!(app.selected_app_id.as_deref(), Some("org.novaos.code"));
        app.back_to_catalog();
        assert_eq!(app.selected_app_id, None);
    }

    #[test]
    fn test_dependency_installation_order() {
        let repo = PackageRepository::default();
        let order = deps::DependencyResolver::resolve_install_order("org.novaos.code", &repo.packages).unwrap();
        assert_eq!(order, vec!["org.novaos.libgit", "org.novaos.code"]);
    }

    #[test]
    fn test_reverse_dependency_block() {
        let mut repo = PackageRepository::default();
        repo.install_package("org.novaos.code").unwrap();
        let block_res = deps::DependencyResolver::check_safe_to_remove("org.novaos.libgit", &repo.installed_db);
        assert!(block_res.is_err(), "Should block removing libgit when code is installed");
    }

    #[test]
    fn test_package_lifecycle_install_update_remove() {
        let mut app = NovaStoreApp::new();
        // 1. Install
        assert!(app.install("org.novaos.notes").is_ok());
        assert!(app.repo.packages.get("org.novaos.notes").unwrap().is_installed());

        // 2. Remove
        assert!(app.remove("org.novaos.notes").is_ok());
        assert!(!app.repo.packages.get("org.novaos.notes").unwrap().is_installed());
    }

    #[test]
    fn test_invalid_signature_rejection() {
        let mut repo = PackageRepository::default();
        if let Some(pkg) = repo.packages.get_mut("org.novaos.notes") {
            pkg.signature_ed25519 = "ed25519:badbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadb".into();
        }
        let install_res = repo.install_package("org.novaos.notes");
        assert!(install_res.is_err(), "Must reject tampered signature");
    }

    #[test]
    fn test_view_rasterization_and_hit_testing() {
        let mut app = NovaStoreApp::new();
        let mut surf = BufferSurface::new(1024, 680);
        app.render(&mut surf);

        let non_zero = surf.pixels.iter().filter(|&&p| p != 0).count();
        assert!(non_zero > 15000, "Should rasterize Store UI elements");

        // Hit test category button (Development is item 1, y ~ 154)
        let clicked = app.handle_click(Point::new(80, 154));
        assert!(clicked);
        assert_eq!(app.active_category, AppCategory::Development);
    }
}
