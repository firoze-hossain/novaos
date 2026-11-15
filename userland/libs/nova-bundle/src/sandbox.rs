//! userland/libs/nova-bundle/src/sandbox.rs
//!
//! Sandboxing and capability profile generation for NovaOS Application Bundles.
//! Translates Manifest.json declared permissions into isolated MAC rules, LD_LIBRARY_PATH
//! private dependency isolation (solving dependency hell), and filesystem access boundaries.

#[cfg(not(test))]
use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[cfg(test)]
use std::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::layout::BundleLayout;
use crate::manifest::BundleManifest;

/// Sandboxed execution profile generated from bundle layout and manifest permissions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxProfile {
    /// Bundle reverse-DNS identifier.
    pub bundle_id: String,
    /// Human-friendly application name.
    pub app_name: String,
    /// Absolute path to primary binary.
    pub executable_path: String,
    /// Isolated bundle root directory.
    pub bundle_dir: String,
    /// Private dependency library path (`<bundle>/lib`), prioritized first to avoid dependency hell.
    pub ld_library_path: String,
    /// Dedicated application data storage directory (isolated from other apps).
    pub data_dir: String,
    /// Dedicated application cache storage directory.
    pub cache_dir: String,
    /// Whitelisted read-only filesystem paths.
    pub read_paths: Vec<String>,
    /// Whitelisted read-write filesystem paths.
    pub write_paths: Vec<String>,
    /// Outbound/inbound network permission flag.
    pub allow_network: bool,
    /// Audio input/output permission flag.
    pub allow_audio: bool,
    /// Camera/video capture permission flag.
    pub allow_camera: bool,
    /// GPU 3D acceleration permission flag.
    pub allow_gpu: bool,
    /// Daemon / background execution permission flag.
    pub allow_background: bool,
    /// Permitted IPC endpoints and service handles.
    pub ipc_channels: Vec<String>,
    /// Complete execution environment variables.
    pub environment: Vec<(String, String)>,
}

impl SandboxProfile {
    /// Generates a locked-down execution profile for an application bundle.
    pub fn generate(layout: &BundleLayout, manifest: &BundleManifest, user_home: &str) -> Self {
        let bundle_dir = layout.root_path.clone();
        let ld_library_path = format!("{}/lib:/lib:/usr/lib", bundle_dir);
        let data_dir = format!("{}/.local/share/{}", user_home.trim_end_matches('/'), manifest.bundle_id);
        let cache_dir = format!("{}/.cache/{}", user_home.trim_end_matches('/'), manifest.bundle_id);
        let exec_path = layout.executable_path(&manifest.executable);

        // Core read paths: bundle itself, system runtime libs, fonts
        let mut read_paths = vec![
            bundle_dir.clone(),
            "/lib".to_string(),
            "/usr/lib".to_string(),
            "/etc/fonts".to_string(),
            "/etc/ssl".to_string(),
        ];
        for rp in &manifest.sandbox.filesystem_read {
            if !read_paths.contains(rp) {
                read_paths.push(rp.clone());
            }
        }

        // Core write paths: isolated app data and cache dirs, session temp
        let mut write_paths = vec![
            data_dir.clone(),
            cache_dir.clone(),
            format!("/tmp/nova-app-{}", manifest.bundle_id),
        ];
        for wp in &manifest.sandbox.filesystem_write {
            if !write_paths.contains(wp) {
                write_paths.push(wp.clone());
            }
        }

        // Permitted IPC channels
        let mut ipc_channels = vec![
            "compositor".to_string(),
            "novad_event".to_string(),
        ];
        if manifest.sandbox.audio {
            ipc_channels.push("audio_mixer".to_string());
        }
        for ipc in &manifest.sandbox.ipc_services {
            if !ipc_channels.contains(ipc) {
                ipc_channels.push(ipc.clone());
            }
        }

        // Synthesize environment variables
        let mut environment = vec![
            ("LD_LIBRARY_PATH".to_string(), ld_library_path.clone()),
            ("NOVA_BUNDLE_DIR".to_string(), bundle_dir.clone()),
            ("NOVA_BUNDLE_ID".to_string(), manifest.bundle_id.clone()),
            ("NOVA_APP_DATA".to_string(), data_dir.clone()),
            ("NOVA_APP_CACHE".to_string(), cache_dir.clone()),
            ("APP_VERSION".to_string(), manifest.version.clone()),
        ];
        for (k, v) in &manifest.env_vars {
            environment.push((k.clone(), v.clone()));
        }

        Self {
            bundle_id: manifest.bundle_id.clone(),
            app_name: manifest.name.clone(),
            executable_path: exec_path,
            bundle_dir,
            ld_library_path,
            data_dir,
            cache_dir,
            read_paths,
            write_paths,
            allow_network: manifest.sandbox.network,
            allow_audio: manifest.sandbox.audio,
            allow_camera: manifest.sandbox.camera,
            allow_gpu: manifest.sandbox.gpu,
            allow_background: manifest.sandbox.background_daemon,
            ipc_channels,
            environment,
        }
    }

    /// Evaluates if a given path is permitted for read access.
    pub fn check_read_permission(&self, target_path: &str) -> bool {
        let norm = target_path.trim_end_matches('/');
        self.read_paths.iter().any(|allowed| norm.starts_with(allowed.trim_end_matches('/')))
            || self.check_write_permission(target_path)
    }

    /// Evaluates if a given path is permitted for write access.
    pub fn check_write_permission(&self, target_path: &str) -> bool {
        let norm = target_path.trim_end_matches('/');
        self.write_paths.iter().any(|allowed| norm.starts_with(allowed.trim_end_matches('/')))
    }

    /// Formats the profile into NovaOS MAC (Mandatory Access Control) policy rules.
    pub fn to_mac_rules(&self) -> Vec<String> {
        let mut rules = Vec::new();
        rules.push(format!("profile {} {{", self.bundle_id));
        rules.push(format!("  target_exec: \"{}\"", self.executable_path));
        rules.push(format!("  allow_network: {}", self.allow_network));
        rules.push(format!("  allow_audio: {}", self.allow_audio));
        rules.push(format!("  allow_gpu: {}", self.allow_gpu));
        rules.push(format!("  allow_camera: {}", self.allow_camera));

        for rp in &self.read_paths {
            rules.push(format!("  allow_fs_read: \"{}\"", rp));
        }
        for wp in &self.write_paths {
            rules.push(format!("  allow_fs_write: \"{}\"", wp));
        }
        for ch in &self.ipc_channels {
            rules.push(format!("  allow_ipc: \"{}\"", ch));
        }
        rules.push("}".to_string());
        rules
    }
}
