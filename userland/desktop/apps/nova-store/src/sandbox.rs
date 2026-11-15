//! userland/desktop/apps/nova-store/src/sandbox.rs
//!
//! Sandbox Permission Declaration and Security Boundaries for NovaOS Packages.

#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(test)]
use std::vec::Vec;

/// Declared sandbox permissions requested by an application bundle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SandboxPermissions {
    /// Full internet and local network socket access.
    pub network_access: bool,
    /// Read/Write access to the user's home directory.
    pub home_directory: bool,
    /// Elevated root or raw filesystem access.
    pub system_root: bool,
    /// Microphone and audio recording input access.
    pub audio_input: bool,
    /// Webcam and camera video capture access.
    pub camera_input: bool,
    /// Direct GPU DRM/KMS hardware acceleration.
    pub hardware_gpu: bool,
    /// Background daemon execution without an open window.
    pub background_service: bool,
}

impl SandboxPermissions {
    pub const fn strict_isolated() -> Self {
        Self {
            network_access: false,
            home_directory: false,
            system_root: false,
            audio_input: false,
            camera_input: false,
            hardware_gpu: true,
            background_service: false,
        }
    }

    pub const fn standard_app() -> Self {
        Self {
            network_access: true,
            home_directory: true,
            system_root: false,
            audio_input: false,
            camera_input: false,
            hardware_gpu: true,
            background_service: false,
        }
    }

    pub const fn system_utility() -> Self {
        Self {
            network_access: true,
            home_directory: true,
            system_root: true,
            audio_input: false,
            camera_input: false,
            hardware_gpu: false,
            background_service: true,
        }
    }

    /// Returns human-readable descriptions of all active permissions for UI inspection.
    pub fn list_declared_permissions(&self) -> Vec<&'static str> {
        let mut list = Vec::new();
        if self.network_access {
            list.push("Network: Full internet and local socket access");
        }
        if self.home_directory {
            list.push("Filesystem: Access to Documents, Downloads and user files");
        }
        if self.system_root {
            list.push("Elevated Privileges: Modify system files and configurations");
        }
        if self.audio_input {
            list.push("Microphone: Record and stream sound input");
        }
        if self.camera_input {
            list.push("Camera: Video capture and image processing");
        }
        if self.hardware_gpu {
            list.push("Hardware: Direct GPU 3D rendering and DMA-BUF surfaces");
        }
        if self.background_service {
            list.push("Startup: Supervised daemon runs in background at boot");
        }
        list
    }

    /// Evaluates if this package runs in an isolated sandbox.
    pub fn is_sandboxed(&self) -> bool {
        !self.system_root
    }
}
