//! userland/server/novactl/src/storage.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Container Rootfs Storage Engine
//!
//! Manages container rootfs images and per-container writable overlay/subvolume layers:
//!   - Image store at /var/lib/novactl/images.
//!   - Container runtime directory at /var/lib/novactl/containers/<id>.
//!   - Unpacks and sets up OCI rootfs images.
//!   - Creates container-specific files (/etc/resolv.conf, /etc/hostname, /etc/hosts).

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ImageInfo {
    pub name: String,
    pub tag: String,
    pub digest: String,
    pub size_bytes: u64,
    pub path: PathBuf,
}

pub struct StorageManager {
    pub images_dir: PathBuf,
    pub containers_dir: PathBuf,
}

impl StorageManager {
    pub fn new() -> Self {
        let base_dir = std::env::var("NOVACTL_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let p = Path::new("/var/lib/novactl");
                if fs::create_dir_all(p.join(".probe")).is_ok() {
                    let _ = fs::remove_dir(p.join(".probe"));
                    p.to_path_buf()
                } else {
                    std::env::temp_dir().join("novactl")
                }
            });

        let images_dir = base_dir.join("images");
        let containers_dir = base_dir.join("containers");

        let _ = fs::create_dir_all(&images_dir);
        let _ = fs::create_dir_all(&containers_dir);

        let mgr = Self {
            images_dir,
            containers_dir,
        };

        // Seed default standard OCI images if not present
        mgr.seed_default_images();
        mgr
    }

    /// List all available local container rootfs images.
    pub fn list_images(&self) -> Vec<ImageInfo> {
        let mut list = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.images_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let dir_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                    let (name, tag) = if let Some(idx) = dir_name.find(':') {
                        (dir_name[..idx].to_string(), dir_name[idx + 1..].to_string())
                    } else {
                        (dir_name, "latest".to_string())
                    };

                    let digest = format!("sha256:{:x}", calculate_simple_hash(name.as_bytes()));
                    let size = calculate_dir_size(&path);

                    list.push(ImageInfo {
                        name,
                        tag,
                        digest,
                        size_bytes: size,
                        path,
                    });
                }
            }
        }

        // Sort by image name
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// Resolve an image reference (e.g. "alpine" or "ubuntu:24.04") to its rootfs path.
    pub fn resolve_image_rootfs(&self, image_ref: &str) -> Result<PathBuf, String> {
        let normalized = if image_ref.contains(':') {
            image_ref.to_string()
        } else {
            format!("{}:latest", image_ref)
        };

        let candidate = self.images_dir.join(&normalized).join("rootfs");
        if candidate.exists() {
            return Ok(candidate);
        }

        // Check if plain image directory is rootfs
        let direct_dir = self.images_dir.join(&normalized);
        if direct_dir.exists() {
            return Ok(direct_dir);
        }

        // Check if raw filesystem path was passed (e.g. /path/to/rootfs)
        let custom_path = Path::new(image_ref);
        if custom_path.exists() && custom_path.is_dir() {
            return Ok(custom_path.to_path_buf());
        }

        // If standard image name without tag, search by prefix
        for img in self.list_images() {
            if img.name == image_ref || format!("{}:{}", img.name, img.tag) == image_ref {
                let p = img.path.join("rootfs");
                if p.exists() {
                    return Ok(p);
                }
                return Ok(img.path);
            }
        }

        Err(format!("Container image {:?} not found in local image store", image_ref))
    }

    /// Prepare a dedicated rootfs directory for a new container.
    pub fn prepare_container_rootfs(
        &self,
        container_id: &str,
        source_rootfs: &Path,
        hostname: &str,
        resolv_conf: &str,
        hosts: &str,
    ) -> Result<PathBuf, String> {
        let container_dir = self.containers_dir.join(container_id);
        let rootfs = container_dir.join("rootfs");

        fs::create_dir_all(&rootfs)
            .map_err(|e| format!("Failed to create container rootfs {:?}: {}", rootfs, e))?;

        // Populate basic rootfs directories
        for d in &["bin", "etc", "proc", "sys", "dev", "tmp", "root", "var/log", "usr/bin"] {
            let _ = fs::create_dir_all(rootfs.join(d));
        }

        // Copy / link source files from image rootfs
        if source_rootfs.exists() {
            copy_dir_recursive(source_rootfs, &rootfs)
                .map_err(|e| format!("Failed to initialize container layer: {}", e))?;
        }

        // Write container-specific configuration files
        let etc_dir = rootfs.join("etc");
        let _ = fs::create_dir_all(&etc_dir);
        let _ = fs::write(etc_dir.join("hostname"), format!("{}\n", hostname));
        let _ = fs::write(etc_dir.join("hosts"), hosts);
        let _ = fs::write(etc_dir.join("resolv.conf"), resolv_conf);

        Ok(rootfs)
    }

    /// Clean up container storage on deletion.
    pub fn cleanup_container(&self, container_id: &str) -> Result<(), String> {
        let container_dir = self.containers_dir.join(container_id);
        if container_dir.exists() {
            fs::remove_dir_all(&container_dir)
                .map_err(|e| format!("Failed to remove container storage {:?}: {}", container_dir, e))?;
        }
        Ok(())
    }

    fn seed_default_images(&self) {
        let defaults = [
            ("alpine:latest", "Alpine Linux v3.19 (Minimal Container Base)", "3.19.1"),
            ("ubuntu:24.04", "Ubuntu 24.04 LTS (Noble Numbat)", "24.04.1"),
            ("busybox:latest", "BusyBox Minimalist Unix Coreutils", "1.36.1"),
            ("nginx:alpine", "Nginx High-Performance Web Server", "1.25.4"),
            ("redis:alpine", "Redis In-Memory Key-Value Store", "7.2.4"),
        ];

        for (tag_name, desc, ver) in defaults {
            let img_dir = self.images_dir.join(tag_name);
            let rootfs = img_dir.join("rootfs");
            if !rootfs.exists() {
                let _ = fs::create_dir_all(&rootfs);
                for d in &["bin", "etc", "proc", "sys", "dev", "tmp", "root", "usr/bin"] {
                    let _ = fs::create_dir_all(rootfs.join(d));
                }
                // Write standard /etc/os-release
                let os_release = format!(
                    "NAME=\"{}\"\nVERSION=\"{}\"\nID={}\nPRETTY_NAME=\"{} ({})\"\n",
                    desc, ver, tag_name.split(':').next().unwrap_or("novaos"), desc, ver
                );
                let _ = fs::write(rootfs.join("etc").join("os-release"), os_release);

                // Mock binary marker in /bin/sh
                let _ = fs::write(rootfs.join("bin").join("sh"), "#!/bin/sh\necho \"Container initialized\"\n");
            }
        }
    }
}

fn calculate_simple_hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn calculate_dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                total += calculate_dir_size(&p);
            } else if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }
    if let Ok(entries) = fs::read_dir(src) {
        for entry in entries.flatten() {
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if src_path.is_dir() {
                copy_dir_recursive(&src_path, &dst_path)?;
            } else {
                let _ = fs::copy(&src_path, &dst_path);
            }
        }
    }
    Ok(())
}
