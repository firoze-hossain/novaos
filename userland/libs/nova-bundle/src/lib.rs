//! userland/libs/nova-bundle/src/lib.rs
//!
//! NovaOS Self-Contained Application Bundle Architecture (`.app` Bundles).
//!
//! Inspired by macOS application bundles and NeXTSTEP architecture, NovaOS application bundles
//! package all metadata, 64-bit ELF binaries, private shared libraries, icons, fonts, and assets
//! into a single directory hierarchy (`AppName.app`).
//!
//! Key Architecture Pillars:
//! 1. Elimination of Dependency Hell: Bundles ship private dependencies in `lib/`, which are
//!    prioritized first in `LD_LIBRARY_PATH` during launch, preventing system library conflicts.
//! 2. Zero-Residual Installation & Deletion: Installing an app requires simply copying or
//!    dropping its `.app` folder into `/Applications`. Uninstalling requires simply deleting it.
//! 3. Declarative Sandboxing: Every app declares capabilities and permission boundaries in
//!    `Manifest.json`, which NovaOS compiles into isolated MAC security rules and memory limits.

#![cfg_attr(not(test), no_std)]

#[cfg(not(test))]
extern crate alloc;

#[cfg(not(test))]
use alloc::{
    format,
    string::{String, ToString},
    vec,
};

#[cfg(test)]
use std::{
    format,
    string::{String, ToString},
    vec,
};

pub mod manifest;
pub mod layout;
pub mod sandbox;
pub mod installer;
pub mod builder;
pub mod scanner;

pub use manifest::{BundleManifest, ManifestError, SandboxPermissions, JsonValue};
pub use layout::{
    BundleFileEntry, BundleLayout, BundleStructure, LayoutError,
    APPLICATIONS_DIR, ASSETS_DIRNAME, BIN_DIRNAME, BUNDLE_EXTENSION,
    LIB_DIRNAME, MANIFEST_FILENAME, SYSTEM_APPS_DIR, USER_APPS_DIR,
};
pub use sandbox::SandboxProfile;
pub use installer::{BundleInstaller, InstallError, UninstallReport};
pub use builder::BundleBuilder;
pub use scanner::BundleScanner;

/// Representation of a validated NovaOS self-contained Application Bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppBundle {
    /// Physical or virtual directory layout.
    pub layout: BundleLayout,
    /// Parsed application manifest (`Manifest.json`).
    pub manifest: BundleManifest,
    /// Internal file tree structure and integrity records.
    pub structure: BundleStructure,
}

impl AppBundle {
    /// Resolves the parent directory containing this bundle.
    pub fn parent_dir(&self) -> String {
        let p = self.layout.root_path.trim_end_matches('/');
        match p.rfind('/') {
            Some(idx) => p[..idx].to_string(),
            None => "/".to_string(),
        }
    }

    /// Resolves the absolute path to the bundle's primary binary.
    pub fn executable_path(&self) -> String {
        self.layout.executable_path(&self.manifest.executable)
    }

    /// Resolves the absolute path to the bundle's main icon.
    pub fn icon_path(&self) -> String {
        self.layout.icon_path(&self.manifest.icon)
    }

    /// Checks if a private shared library exists in `lib/` (solving dependency conflicts).
    pub fn has_private_library(&self, lib_filename: &str) -> bool {
        let clean = lib_filename.trim_start_matches('/');
        let rel = format!("lib/{}", clean);
        self.structure.has_file(&rel)
    }

    /// Verifies bundle internal consistency against declared manifest.
    pub fn verify(&self) -> Result<(), LayoutError> {
        self.structure.verify_integrity(&self.manifest)
    }

    /// Generates isolated runtime sandbox execution profile.
    pub fn generate_sandbox_profile(&self, user_home: &str) -> SandboxProfile {
        SandboxProfile::generate(&self.layout, &self.manifest, user_home)
    }
}

/// 10-point self-test suite for NovaOS Application Bundle Subsystem.
pub fn run_nova_bundle_selftest() -> bool {
    // 1. Manifest JSON serialization and roundtrip parsing
    let mut manifest = BundleManifest::new(
        "org.novaos.calculator",
        "Nova Calculator",
        "2.1.0",
        "bin/calculator",
    );
    manifest.description = "Precision scientific and graphing calculator".to_string();
    manifest.categories = vec!["Productivity".to_string(), "Utilities".to_string()];
    manifest.sandbox.audio = true;
    manifest.sandbox.gpu = true;
    manifest.sandbox.filesystem_read = vec!["/users/root/Documents".to_string()];
    manifest.sandbox.filesystem_write = vec!["/users/root/Documents/Calculations".to_string()];
    manifest.sandbox.ipc_services = vec!["math_engine".to_string()];
    manifest.env_vars.push(("CALC_PRECISION".to_string(), "64".to_string()));
    manifest.mime_types.push("application/x-nova-calc".to_string());

    let json_text = manifest.to_json();
    let parsed_manifest = match BundleManifest::from_json(&json_text) {
        Ok(m) => m,
        Err(_) => return false,
    };
    if parsed_manifest.bundle_id != "org.novaos.calculator"
        || parsed_manifest.name != "Nova Calculator"
        || !parsed_manifest.sandbox.audio
        || !parsed_manifest.sandbox.gpu
        || parsed_manifest.sandbox.filesystem_read.len() != 1
        || parsed_manifest.env_vars.len() != 1
    {
        return false;
    }

    // 2. Bundle Layout and Path Resolution
    let layout = match BundleLayout::from_path("/Applications/Calculator.app") {
        Ok(l) => l,
        Err(_) => return false,
    };
    if layout.folder_name != "Calculator.app"
        || layout.manifest_path() != "/Applications/Calculator.app/Manifest.json"
        || layout.bin_dir() != "/Applications/Calculator.app/bin"
        || layout.assets_dir() != "/Applications/Calculator.app/assets"
        || layout.lib_dir() != "/Applications/Calculator.app/lib"
        || layout.executable_path("bin/calc") != "/Applications/Calculator.app/bin/calc"
        || layout.icon_path("assets/icon.png") != "/Applications/Calculator.app/assets/icon.png"
    {
        return false;
    }

    // 3. Programmatic Bundle Construction via BundleBuilder
    let bundle = match BundleBuilder::new("org.novaos.calculator", "Nova Calculator", "2.1.0")
        .description("Precision scientific calculator")
        .author("NovaOS Architecture Team")
        .categories(vec!["Productivity".to_string()])
        .permissions(SandboxPermissions {
            network: false,
            audio: true,
            camera: false,
            gpu: true,
            filesystem_read: vec!["/users/root/Documents".to_string()],
            filesystem_write: vec!["/users/root/.local/share/calc".to_string()],
            background_daemon: false,
            ipc_services: vec!["math_daemon".to_string()],
        })
        .set_primary_executable("calculator", 524288) // 512KB ELF binary
        .set_main_icon("icon_256.png", 65536)        // 64KB icon
        .add_shared_library("libmpfr.so", 2097152)   // 2MB private math library (Dependency Hell solved!)
        .add_asset("presets.dat", 4096)
        .env_var("NOVA_THEME", "dark")
        .mime_type("application/x-calc")
        .build("/Applications/Calculator.app")
    {
        Ok(b) => b,
        Err(_) => return false,
    };

    // 4. Structural Integrity & Component Verification
    if bundle.verify().is_err() {
        return false;
    }
    if !bundle.has_private_library("libmpfr.so") {
        return false;
    }
    if bundle.structure.total_size_bytes() < 2500000 {
        return false;
    }

    // 5. Sandbox Profile Generation & Dependency Hell Isolation
    let profile = bundle.generate_sandbox_profile("/users/root");
    if profile.bundle_id != "org.novaos.calculator" {
        return false;
    }
    // Verify LD_LIBRARY_PATH prioritizes bundle private /lib ahead of system /lib!
    if !profile.ld_library_path.starts_with("/Applications/Calculator.app/lib:") {
        return false;
    }
    if !profile.allow_audio || !profile.allow_gpu || profile.allow_network {
        return false;
    }

    // 6. Security Boundary Enforcement
    if !profile.check_read_permission("/Applications/Calculator.app") {
        return false;
    }
    if !profile.check_read_permission("/lib") {
        return false;
    }
    if !profile.check_read_permission("/users/root/Documents") {
        return false;
    }
    // Should NOT have read permission to sensitive kernel or other apps' private stores
    if profile.check_read_permission("/etc/shadow") {
        return false;
    }
    if profile.check_read_permission("/users/other_user/Private") {
        return false;
    }
    if !profile.check_write_permission("/users/root/.local/share/calc") {
        return false;
    }
    if profile.check_write_permission("/bin/sh") {
        return false;
    }

    // 7. Drag-and-Drop Installation into User Applications Directory
    let installed = match BundleInstaller::install_bundle(&bundle, "/users/root/Applications", true) {
        Ok(ib) => ib,
        Err(_) => return false,
    };
    if installed.layout.root_path != "/users/root/Applications/Calculator.app" {
        return false;
    }
    if installed.verify().is_err() {
        return false;
    }

    // 8. Atomic Upgrade Verification
    let upgraded_bundle = match BundleBuilder::new("org.novaos.calculator", "Nova Calculator", "2.2.0")
        .set_primary_executable("calculator", 600000)
        .set_main_icon("icon_256.png", 65536)
        .build("/tmp/Calculator.app")
    {
        Ok(b) => b,
        Err(_) => return false,
    };
    let upgraded_result = match BundleInstaller::upgrade_bundle(&upgraded_bundle, &installed) {
        Ok(u) => u,
        Err(_) => return false,
    };
    if upgraded_result.manifest.version != "2.2.0" {
        return false;
    }

    // 9. Clean Uninstallation (Zero Orphaned Residuals)
    let uninstall_report = match BundleInstaller::uninstall_bundle(&installed, true, "/users/root") {
        Ok(rep) => rep,
        Err(_) => return false,
    };
    if uninstall_report.bundle_id != "org.novaos.calculator"
        || uninstall_report.files_removed == 0
        || uninstall_report.bytes_reclaimed == 0
        || !uninstall_report.app_data_purged
    {
        return false;
    }

    // 10. Bundle Discovery, Search, and Category Indexing
    let mut scanner = BundleScanner::new();
    scanner.register_bundle(bundle.clone());

    let edit_bundle = match BundleBuilder::new("org.novaos.editor", "Nova Code Studio", "1.0.0")
        .description("Professional IDE and code editor")
        .categories(vec!["Development".to_string()])
        .set_primary_executable("nova-edit", 800000)
        .set_main_icon("icon.png", 32768)
        .build("/Applications/NovaEdit.app")
    {
        Ok(b) => b,
        Err(_) => return false,
    };
    scanner.register_bundle(edit_bundle);

    if scanner.installed_bundles.len() != 2 {
        return false;
    }

    let search_results = scanner.search("calculator");
    if search_results.len() != 1 || search_results[0].manifest.name != "Nova Calculator" {
        return false;
    }

    let dev_apps = scanner.find_by_category("Development");
    if dev_apps.len() != 1 || dev_apps[0].manifest.bundle_id != "org.novaos.editor" {
        return false;
    }

    let cats = scanner.all_categories();
    if !cats.contains(&"Productivity".to_string()) || !cats.contains(&"Development".to_string()) {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_serialization() {
        let mut m = BundleManifest::new("org.test.app", "Test App", "1.0.0", "bin/app");
        m.categories = vec!["Utilities".to_string()];
        let json = m.to_json();
        assert!(json.contains("\"bundle_id\": \"org.test.app\""));
        assert!(json.contains("\"executable\": \"bin/app\""));

        let roundtrip = BundleManifest::from_json(&json).expect("should parse");
        assert_eq!(roundtrip.bundle_id, "org.test.app");
        assert_eq!(roundtrip.name, "Test App");
    }

    #[test]
    fn test_manifest_validation() {
        let invalid = BundleManifest::new("invalid_no_dot", "App", "1.0.0", "bin/app");
        assert!(invalid.validate().is_err());

        let invalid_exec = BundleManifest::new("org.app", "App", "1.0.0", "usr/bin/app");
        assert!(invalid_exec.validate().is_err());
    }

    #[test]
    fn test_bundle_layout_paths() {
        let layout = BundleLayout::from_path("/System/Applications/Files.app").unwrap();
        assert_eq!(layout.folder_name, "Files.app");
        assert_eq!(layout.manifest_path(), "/System/Applications/Files.app/Manifest.json");
        assert_eq!(layout.bin_dir(), "/System/Applications/Files.app/bin");
        assert_eq!(layout.assets_dir(), "/System/Applications/Files.app/assets");
        assert_eq!(layout.lib_dir(), "/System/Applications/Files.app/lib");
    }

    #[test]
    fn test_bundle_builder_and_verify() {
        let bundle = BundleBuilder::new("org.novaos.term", "Nova Terminal", "2.0.0")
            .set_primary_executable("nova-term", 100000)
            .set_main_icon("icon.png", 20000)
            .add_shared_library("libvte.so", 50000)
            .build("/Applications/Terminal.app")
            .unwrap();

        assert_eq!(bundle.manifest.bundle_id, "org.novaos.term");
        assert!(bundle.verify().is_ok());
        assert!(bundle.has_private_library("libvte.so"));
    }

    #[test]
    fn test_sandbox_profile_generation() {
        let bundle = BundleBuilder::new("org.novaos.browser", "Nova Web", "1.0.0")
            .permissions(SandboxPermissions {
                network: true,
                audio: true,
                camera: false,
                gpu: true,
                filesystem_read: vec!["/users/root/Downloads".to_string()],
                filesystem_write: vec!["/users/root/Downloads".to_string()],
                background_daemon: false,
                ipc_services: vec![],
            })
            .set_primary_executable("browser", 200000)
            .set_main_icon("icon.png", 30000)
            .build("/Applications/Browser.app")
            .unwrap();

        let profile = bundle.generate_sandbox_profile("/users/root");
        assert!(profile.allow_network);
        assert!(profile.allow_gpu);
        assert!(!profile.allow_camera);
        assert!(profile.ld_library_path.starts_with("/Applications/Browser.app/lib:"));
        assert!(profile.check_read_permission("/users/root/Downloads"));
        assert!(!profile.check_read_permission("/kernel/secrets"));
    }

    #[test]
    fn test_installer_drag_and_drop() {
        let bundle = BundleBuilder::new("org.novaos.music", "Nova Music", "1.0.0")
            .set_primary_executable("music", 150000)
            .set_main_icon("icon.png", 25000)
            .build("/tmp/Music.app")
            .unwrap();

        let installed = BundleInstaller::install_bundle(&bundle, "/Applications", false).unwrap();
        assert_eq!(installed.layout.root_path, "/Applications/Music.app");
        assert!(installed.verify().is_ok());
    }

    #[test]
    fn test_clean_uninstallation() {
        let bundle = BundleBuilder::new("org.novaos.notes", "Nova Notes", "1.0.0")
            .set_primary_executable("notes", 80000)
            .set_main_icon("icon.png", 10000)
            .build("/Applications/Notes.app")
            .unwrap();

        let report = BundleInstaller::uninstall_bundle(&bundle, true, "/users/root").unwrap();
        assert_eq!(report.bundle_id, "org.novaos.notes");
        assert!(report.files_removed >= 3);
        assert!(report.bytes_reclaimed > 0);
    }

    #[test]
    fn test_scanner_search_and_filter() {
        let mut scanner = BundleScanner::new();
        let app1 = BundleBuilder::new("org.novaos.chat", "Nova Chat", "1.0.0")
            .categories(vec!["Internet".to_string()])
            .set_primary_executable("chat", 100000)
            .set_main_icon("icon.png", 10000)
            .build("/Applications/Chat.app")
            .unwrap();
        let app2 = BundleBuilder::new("org.novaos.paint", "Nova Paint", "1.0.0")
            .categories(vec!["Creativity".to_string()])
            .set_primary_executable("paint", 100000)
            .set_main_icon("icon.png", 10000)
            .build("/Applications/Paint.app")
            .unwrap();

        scanner.register_bundle(app1);
        scanner.register_bundle(app2);

        assert_eq!(scanner.find_by_category("Internet").len(), 1);
        assert_eq!(scanner.search("paint").len(), 1);
        assert_eq!(scanner.search("nova").len(), 2);
    }

    #[test]
    fn test_selftest_passes() {
        assert!(run_nova_bundle_selftest());
    }
}
