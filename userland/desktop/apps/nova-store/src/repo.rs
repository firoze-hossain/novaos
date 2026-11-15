//! userland/desktop/apps/nova-store/src/repo.rs
//!
//! Repository Index Engine, Package Signatures, and Lifecycle Management.

#[cfg(not(test))]
use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[cfg(test)]
use std::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::crypto::{sha256, sign_package, verify_package_signature};
use crate::deps::DependencyResolver;
use crate::package::{AppCategory, AppPackage, InstallStatus, ScreenshotPreview};
use crate::sandbox::SandboxPermissions;

/// Package Repository coordinator.
#[derive(Clone, Debug)]
pub struct PackageRepository {
    pub repo_url: String,
    pub packages: BTreeMap<String, AppPackage>,
    pub installed_db: BTreeMap<String, AppPackage>,
    pub audit_log: Vec<String>,
}

impl Default for PackageRepository {
    fn default() -> Self {
        Self::new("https://repo.novaos.org/v2")
    }
}

impl PackageRepository {
    pub fn new(repo_url: &str) -> Self {
        let mut repo = Self {
            repo_url: repo_url.to_string(),
            packages: BTreeMap::new(),
            installed_db: BTreeMap::new(),
            audit_log: Vec::new(),
        };
        repo.populate_catalog();
        repo
    }

    /// Populates repository catalogue with officially signed packages.
    fn populate_catalog(&mut self) {
        // 1. Nova Code Studio (Development)
        self.register_signed_package(
            "org.novaos.code",
            "Nova Code Studio",
            "2.4.0",
            AppCategory::Development,
            "NovaOS Developer Tools Team",
            "MIT / Apache-2.0",
            "Modern, distraction-free code editor and IDE with native Rust and C tooling",
            "Nova Code Studio provides blazing-fast editing with syntax highlighting, inline Git diffs, integrated terminal, and sub-millisecond search across large source trees.",
            "Added GPU-accelerated glyph caching and integrated rust-analyzer language server bridge.",
            "[IDE]",
            48 * 1024 * 1024,
            120 * 1024 * 1024,
            4.9,
            1420,
            vec!["org.novaos.libgit".into()],
            SandboxPermissions::standard_app(),
            vec![
                ScreenshotPreview {
                    title: "Editor Workspace".into(),
                    caption: "Clean editing view with code tree and terminal".into(),
                    accent_rgb: (0, 120, 212),
                },
                ScreenshotPreview {
                    title: "Git Integration".into(),
                    caption: "Visual diffing and branch graph viewer".into(),
                    accent_rgb: (94, 92, 230),
                },
            ],
            InstallStatus::NotInstalled,
        );

        // 2. Nova Git Core Library (Shared Dependency)
        self.register_signed_package(
            "org.novaos.libgit",
            "Nova Git Core Library",
            "1.0.0",
            AppCategory::Development,
            "NovaOS Architecture Team",
            "LGPL-2.1",
            "Shared native library for Git object storage and protocol operations",
            "Provides pure Rust high-efficiency Git repository parsing, commit history analysis, and packfile decompression.",
            "Initial release with zero-copy packfile indexing.",
            "[LIB]",
            8 * 1024 * 1024,
            16 * 1024 * 1024,
            4.8,
            850,
            vec![],
            SandboxPermissions::strict_isolated(),
            vec![],
            InstallStatus::NotInstalled,
        );

        // 3. Nova Scientific Calculator (Productivity)
        self.register_signed_package(
            "org.novaos.calc",
            "Nova Calculator",
            "2.0.0",
            AppCategory::Productivity,
            "NovaOS Apps Team",
            "GPL-3.0",
            "Symbolic calculator, unit converter, and currency exchange tools",
            "Comprehensive calculation suite supporting standard arithmetic, scientific functions, programmer hex/bin logic, and instant unit conversions.",
            "Added currency conversion API integration and matrix operations.",
            "[CALC]",
            4 * 1024 * 1024,
            10 * 1024 * 1024,
            4.7,
            930,
            vec![],
            SandboxPermissions::strict_isolated(),
            vec![
                ScreenshotPreview {
                    title: "Scientific View".into(),
                    caption: "Trigonometric and logarithmic formulas".into(),
                    accent_rgb: (48, 209, 88),
                },
            ],
            InstallStatus::Installed { version: "2.0.0".into() },
        );

        // 4. Nova Notes (Productivity)
        self.register_signed_package(
            "org.novaos.notes",
            "Nova Notes",
            "1.5.0",
            AppCategory::Productivity,
            "NovaOS Apps Team",
            "MIT",
            "Distraction-free markdown notebook with instant live preview and tags",
            "Organize your thoughts, meeting notes, and technical drafts with hierarchical tagging, fast full-text search, and math expression support.",
            "Added KaTeX formula preview and auto-saving snapshots.",
            "[NOTE]",
            12 * 1024 * 1024,
            28 * 1024 * 1024,
            4.8,
            1120,
            vec![],
            SandboxPermissions::standard_app(),
            vec![
                ScreenshotPreview {
                    title: "Dual Pane Markdown".into(),
                    caption: "Side-by-side editing and formatted presentation".into(),
                    accent_rgb: (255, 159, 10),
                },
            ],
            InstallStatus::NotInstalled,
        );

        // 5. Nova Activity Monitor (Utilities)
        self.register_signed_package(
            "org.novaos.sysmon",
            "Nova Activity Monitor",
            "2.0.0",
            AppCategory::Utilities,
            "NovaOS System Team",
            "GPL-3.0",
            "Real-time CPU, RAM, GPU, storage and network process telemetry",
            "Monitor system performance with high-frequency CPU utilization graphs, memory allocation breakdowns, disk I/O rates, and process termination controls.",
            "Integrated VirtIO-GPU 3D load tracker and cgroups v2 resource metrics.",
            "[STAT]",
            6 * 1024 * 1024,
            14 * 1024 * 1024,
            4.9,
            2100,
            vec![],
            SandboxPermissions::system_utility(),
            vec![
                ScreenshotPreview {
                    title: "Performance Graphs".into(),
                    caption: "Real-time multi-core CPU load and memory usage".into(),
                    accent_rgb: (255, 55, 95),
                },
            ],
            InstallStatus::Installed { version: "1.9.0".into() }, // Updatable!
        );

        // 6. Nova Archive Manager (Utilities)
        self.register_signed_package(
            "org.novaos.archive",
            "Nova Archive Manager",
            "2.1.0",
            AppCategory::Utilities,
            "NovaOS System Team",
            "BSD-2-Clause",
            "High-speed archive extraction and compression for tar, zip, and gz",
            "Create and extract archives with multi-threaded LZ4 and DEFLATE compression, drag-and-drop file inspection, and integrity checksum verification.",
            "Added 7z format support and password-protected zip archives.",
            "[ZIP]",
            5 * 1024 * 1024,
            12 * 1024 * 1024,
            4.6,
            640,
            vec![],
            SandboxPermissions::standard_app(),
            vec![],
            InstallStatus::NotInstalled,
        );

        // 7. Nova Web Browser (Internet)
        self.register_signed_package(
            "org.novaos.browser",
            "Nova Web Browser",
            "2.0.0",
            AppCategory::Internet,
            "NovaOS Internet Team",
            "MPL-2.0",
            "Fast, private web browser with built-in ad blocking and sandbox security",
            "Navigate the modern web with tabs, privacy shields, hardware-accelerated page compositing, and integrated downloads manager.",
            "Updated layout engine with WebAssembly support and strict site isolation.",
            "[WEB]",
            64 * 1024 * 1024,
            180 * 1024 * 1024,
            4.8,
            3400,
            vec![],
            SandboxPermissions::standard_app(),
            vec![
                ScreenshotPreview {
                    title: "Tabbed Browsing".into(),
                    caption: "Fast, hardware-accelerated modern web navigation".into(),
                    accent_rgb: (0, 120, 212),
                },
            ],
            InstallStatus::NotInstalled,
        );

        // 8. Nova Matrix Chat (Internet)
        self.register_signed_package(
            "org.novaos.chat",
            "Nova Matrix Chat",
            "1.2.0",
            AppCategory::Internet,
            "NovaOS Security Team",
            "GPL-3.0",
            "End-to-end encrypted messaging, voice calls, and team collaboration",
            "Communicate securely with decentralized Matrix protocol, cross-device verification, message search, and shared file spaces.",
            "Added encrypted voice room calling and markdown message formatting.",
            "[MSG]",
            22 * 1024 * 1024,
            55 * 1024 * 1024,
            4.7,
            1540,
            vec![],
            SandboxPermissions::standard_app(),
            vec![
                ScreenshotPreview {
                    title: "Encrypted Room".into(),
                    caption: "End-to-end encrypted discussion channel".into(),
                    accent_rgb: (191, 90, 242),
                },
            ],
            InstallStatus::NotInstalled,
        );

        // Initialize installed_db based on initial installed status
        for (id, pkg) in &self.packages {
            if pkg.is_installed() {
                self.installed_db.insert(id.clone(), pkg.clone());
            }
        }
    }

    /// Registers a package with a mathematically valid Ed25519 signature.
    #[allow(clippy::too_many_arguments)]
    fn register_signed_package(
        &mut self,
        id: &str,
        name: &str,
        version: &str,
        category: AppCategory,
        developer: &str,
        license: &str,
        summary: &str,
        description: &str,
        changelog: &str,
        icon_symbol: &'static str,
        download_size_bytes: u64,
        installed_size_bytes: u64,
        rating: f32,
        rating_count: u32,
        dependencies: Vec<String>,
        permissions: SandboxPermissions,
        screenshots: Vec<ScreenshotPreview>,
        initial_status: InstallStatus,
    ) {
        let payload_dummy = format!("{}-{}-{}", id, version, summary);
        let hash = sha256(payload_dummy.as_bytes());
        let hash_str = crate::crypto::to_hex_str(&hash);
        let signature = sign_package(id, version, &hash);

        // Check if an update is available compared to initial_status
        let effective_status = match &initial_status {
            InstallStatus::Installed { version: inst_ver } => {
                if inst_ver != version {
                    InstallStatus::Updatable {
                        installed_version: inst_ver.clone(),
                        new_version: version.to_string(),
                    }
                } else {
                    initial_status
                }
            }
            _ => initial_status,
        };

        self.packages.insert(
            id.to_string(),
            AppPackage {
                id: id.to_string(),
                name: name.to_string(),
                version: version.to_string(),
                category,
                developer: developer.to_string(),
                license: license.to_string(),
                summary: summary.to_string(),
                description: description.to_string(),
                changelog: changelog.to_string(),
                icon_symbol,
                download_size_bytes,
                installed_size_bytes,
                rating,
                rating_count,
                dependencies,
                permissions,
                sha256_hash: hash_str,
                signature_ed25519: signature,
                status: effective_status,
                screenshots,
            },
        );
    }

    /// Returns packages matching a given category.
    pub fn get_by_category(&self, cat: AppCategory) -> Vec<&AppPackage> {
        match cat {
            AppCategory::Featured => self.packages.values().take(6).collect(),
            AppCategory::Installed => self.packages.values().filter(|p| p.is_installed()).collect(),
            AppCategory::Updates => self.packages.values().filter(|p| p.is_updatable()).collect(),
            _ => self.packages.values().filter(|p| p.category == cat).collect(),
        }
    }

    /// Searches packages across name, summary, description, and developer.
    pub fn search(&self, query: &str) -> Vec<&AppPackage> {
        let clean = query.trim().to_lowercase();
        if clean.is_empty() {
            return Vec::new();
        }

        self.packages
            .values()
            .filter(|p| {
                p.name.to_lowercase().contains(&clean)
                    || p.summary.to_lowercase().contains(&clean)
                    || p.description.to_lowercase().contains(&clean)
                    || p.developer.to_lowercase().contains(&clean)
            })
            .collect()
    }

    /// Performs one-click installation: resolves dependencies, verifies signatures, and installs.
    pub fn install_package(&mut self, id: &str) -> Result<Vec<String>, &'static str> {
        let install_order = DependencyResolver::resolve_install_order(id, &self.packages)
            .map_err(|_| "Failed to resolve dependencies")?;

        let mut installed_list = Vec::new();

        for pkg_id in &install_order {
            let (pkg_ver, pkg_name) = {
                let pkg = self.packages.get(pkg_id).ok_or("Package not found")?;

                // 1. Cryptographic signature and integrity verification
                let payload_dummy = format!("{}-{}-{}", pkg.id, pkg.version, pkg.summary);
                let hash = sha256(payload_dummy.as_bytes());
                if !verify_package_signature(&pkg.id, &pkg.version, &hash, &pkg.signature_ed25519) {
                    self.audit_log.push(format!("REJECTED: {} (Invalid signature)", pkg_id));
                    return Err("Cryptographic signature verification failed");
                }
                (pkg.version.clone(), pkg.name.clone())
            };

            // 2. Install package
            if let Some(p) = self.packages.get_mut(pkg_id) {
                p.status = InstallStatus::Installed {
                    version: p.version.clone(),
                };
                self.installed_db.insert(pkg_id.clone(), p.clone());
                installed_list.push(pkg_name);
            }

            self.audit_log.push(format!("INSTALLED: {} v{}", pkg_id, pkg_ver));
        }

        Ok(installed_list)
    }

    /// One-click update for an installed package.
    pub fn update_package(&mut self, id: &str) -> Result<(), &'static str> {
        let pkg = self.packages.get(id).ok_or("Package not found")?;
        let payload_dummy = format!("{}-{}-{}", pkg.id, pkg.version, pkg.summary);
        let hash = sha256(payload_dummy.as_bytes());

        if !verify_package_signature(&pkg.id, &pkg.version, &hash, &pkg.signature_ed25519) {
            return Err("Cryptographic signature verification failed");
        }

        if let Some(p) = self.packages.get_mut(id) {
            p.status = InstallStatus::Installed {
                version: p.version.clone(),
            };
            self.installed_db.insert(id.to_string(), p.clone());
        }

        self.audit_log.push(format!("UPDATED: {}", id));
        Ok(())
    }

    /// One-click uninstallation with reverse dependency safety check.
    pub fn remove_package(&mut self, id: &str) -> Result<(), String> {
        // Check reverse dependencies
        if let Err(blockers) = DependencyResolver::check_safe_to_remove(id, &self.installed_db) {
            return Err(format!(
                "Cannot remove package: required by {}",
                blockers.join(", ")
            ));
        }

        if let Some(p) = self.packages.get_mut(id) {
            p.status = InstallStatus::NotInstalled;
            self.installed_db.remove(id);
        }

        self.audit_log.push(format!("REMOVED: {}", id));
        Ok(())
    }
}
