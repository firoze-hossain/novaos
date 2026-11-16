//! userland/server/novactl/src/namespace.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Namespace Isolation Setup
//!
//! Manages Linux-grade namespaces for container processes:
//!   - PID Namespace: container init process runs as local PID 1.
//!   - Mount Namespace: isolated mount table & normalized chroot jail.
//!   - Network Namespace: private loopback and veth interfaces.
//!   - IPC Namespace: independent message queues and shared memory.
//!   - UTS Namespace: isolated nodename / hostname.
//!   - User Namespace: UID/GID translation to unprivileged host user.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

// Standard Linux Clone Flags
pub const CLONE_NEWNS: u32     = 0x0002_0000; // Mount namespace
pub const CLONE_NEWCGROUP: u32 = 0x0200_0000; // Cgroup namespace
pub const CLONE_NEWUTS: u32    = 0x0400_0000; // UTS namespace
pub const CLONE_NEWIPC: u32    = 0x0800_0000; // IPC namespace
pub const CLONE_NEWUSER: u32   = 0x1000_0000; // User namespace
pub const CLONE_NEWPID: u32    = 0x2000_0000; // PID namespace
pub const CLONE_NEWNET: u32    = 0x4000_0000; // Network namespace

#[derive(Clone, Debug)]
pub struct NamespaceConfig {
    pub hostname: String,
    pub rootfs_path: PathBuf,
    pub uid_mappings: Vec<(u32, u32, u32)>, // (inside_uid, outside_uid, count)
    pub gid_mappings: Vec<(u32, u32, u32)>,
    pub enable_all_namespaces: bool,
}

impl NamespaceConfig {
    pub fn new(hostname: &str, rootfs_path: &Path) -> Self {
        Self {
            hostname: hostname.to_string(),
            rootfs_path: rootfs_path.to_path_buf(),
            uid_mappings: vec![(0, 1000, 1)], // Map host UID 1000 to container root UID 0
            gid_mappings: vec![(0, 1000, 1)],
            enable_all_namespaces: true,
        }
    }

    /// Return bitmask of clone flags needed for complete container isolation.
    pub fn clone_flags(&self) -> u32 {
        if self.enable_all_namespaces {
            CLONE_NEWPID | CLONE_NEWNS | CLONE_NEWNET | CLONE_NEWIPC | CLONE_NEWUTS | CLONE_NEWUSER
        } else {
            CLONE_NEWPID | CLONE_NEWNS
        }
    }

    /// Normalize path and ensure it does not escape the chroot jail boundary.
    pub fn resolve_in_jail(&self, requested_path: &str) -> Result<PathBuf, String> {
        let mut components = Vec::new();
        for comp in Path::new(requested_path).components() {
            match comp {
                std::path::Component::Normal(c) => components.push(c),
                std::path::Component::ParentDir => {
                    // Prevent escaping root
                    components.pop();
                }
                std::path::Component::CurDir | std::path::Component::RootDir => {}
                _ => {}
            }
        }

        let mut resolved = self.rootfs_path.clone();
        for comp in components {
            resolved.push(comp);
        }

        // Verify it remains inside rootfs
        if !resolved.starts_with(&self.rootfs_path) {
            return Err(format!("Path traversal escape attempt detected: {:?}", requested_path));
        }

        Ok(resolved)
    }

    /// Prepare standard pseudo-filesystem mounts within container rootfs.
    pub fn prepare_mount_points(&self) -> Result<(), String> {
        let proc_dir = self.rootfs_path.join("proc");
        let sys_dir = self.rootfs_path.join("sys");
        let dev_dir = self.rootfs_path.join("dev");
        let tmp_dir = self.rootfs_path.join("tmp");

        for d in &[&proc_dir, &sys_dir, &dev_dir, &tmp_dir] {
            std::fs::create_dir_all(d)
                .map_err(|e| format!("Failed to create mount point {:?}: {}", d, e))?;
        }

        Ok(())
    }
}
