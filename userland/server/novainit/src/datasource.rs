//! userland/server/novainit/src/datasource.rs
//!
//! Multi-Provider Cloud-Init Datasource Discovery Engine.
//! Supports: NoCloud, Config-Drive (OpenStack / Proxmox / KVM), Kernel cmdline, and local seed directory.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataSourceType {
    NoCloud,
    ConfigDrive,
    Cmdline,
    LocalSeed,
    None,
}

#[derive(Debug, Clone, Default)]
pub struct RawCloudData {
    pub source_type: String,
    pub meta_data: Option<String>,
    pub user_data: Option<String>,
    pub network_config: Option<String>,
    pub vendor_data: Option<String>,
}

pub struct DataSourceManager;

impl DataSourceManager {
    /// Attempts discovery across all supported cloud datasources in priority order
    pub fn discover(root_dir: &Path, custom_seed: Option<&str>) -> Option<RawCloudData> {
        // 1. Explicit CLI / Environment override
        if let Some(seed) = custom_seed {
            let seed_path = if Path::new(seed).is_absolute() {
                PathBuf::from(seed)
            } else {
                root_dir.join(seed)
            };
            if let Some(data) = Self::load_from_directory(&seed_path, "CustomSeed") {
                return Some(data);
            }
            if let Some(data) = Self::load_from_directory(Path::new(seed), "CustomSeed") {
                return Some(data);
            }
        }
        if let Ok(env_seed) = std::env::var("NOVAINIT_SEED_DIR") {
            let env_path = if Path::new(&env_seed).is_absolute() {
                PathBuf::from(&env_seed)
            } else {
                root_dir.join(&env_seed)
            };
            if let Some(data) = Self::load_from_directory(&env_path, "EnvSeed") {
                return Some(data);
            }
        }

        // 2. Kernel Command Line (`/proc/cmdline`)
        if let Some(data) = Self::discover_cmdline() {
            return Some(data);
        }

        // 3. NoCloud standard seed paths (scoped to root_dir)
        let nocloud_subpaths = [
            "var/lib/cloud/seed/nocloud",
            "var/lib/cloud/seed",
            "var/lib/cloud/instance",
            "etc/cloud/seed",
        ];
        for p in &nocloud_subpaths {
            let target = root_dir.join(p);
            if let Some(data) = Self::load_from_directory(&target, "NoCloud") {
                return Some(data);
            }
        }

        // Also check absolute path fallback if root_dir is not /
        if root_dir != Path::new("/") {
            for p in &nocloud_subpaths {
                let target = Path::new("/").join(p);
                if let Some(data) = Self::load_from_directory(&target, "NoCloud") {
                    return Some(data);
                }
            }
        }

        // 4. Config-Drive paths (OpenStack / Proxmox / KVM)
        let configdrive_subpaths = [
            "media/configdrive",
            "mnt/configdrive",
            "var/lib/cloud/config-drive",
            "run/configdrive",
        ];
        for p in &configdrive_subpaths {
            let target = root_dir.join(p);
            if let Some(data) = Self::load_config_drive(&target) {
                return Some(data);
            }
        }

        if root_dir != Path::new("/") {
            for p in &configdrive_subpaths {
                let target = Path::new("/").join(p);
                if let Some(data) = Self::load_config_drive(&target) {
                    return Some(data);
                }
            }
        }

        None
    }

    /// Loads NoCloud standard files (meta-data, user-data, network-config, vendor-data) from a directory
    pub fn load_from_directory(dir: &Path, source_label: &str) -> Option<RawCloudData> {
        if !dir.is_dir() {
            return None;
        }

        let user_data = Self::read_file_if_exists(&dir.join("user-data"))
            .or_else(|| Self::read_file_if_exists(&dir.join("user-data.yaml")))
            .or_else(|| Self::read_file_if_exists(&dir.join("user_data")));

        let meta_data = Self::read_file_if_exists(&dir.join("meta-data"))
            .or_else(|| Self::read_file_if_exists(&dir.join("meta-data.yaml")))
            .or_else(|| Self::read_file_if_exists(&dir.join("meta_data.json")))
            .or_else(|| Self::read_file_if_exists(&dir.join("meta_data")));

        let network_config = Self::read_file_if_exists(&dir.join("network-config"))
            .or_else(|| Self::read_file_if_exists(&dir.join("network-config.yaml")))
            .or_else(|| Self::read_file_if_exists(&dir.join("network_data.json")));

        let vendor_data = Self::read_file_if_exists(&dir.join("vendor-data"));

        if user_data.is_some() || meta_data.is_some() || network_config.is_some() {
            Some(RawCloudData {
                source_type: source_label.to_string(),
                meta_data,
                user_data,
                network_config,
                vendor_data,
            })
        } else {
            None
        }
    }

    /// Loads OpenStack / Proxmox Config-Drive structure
    pub fn load_config_drive(root: &Path) -> Option<RawCloudData> {
        if !root.is_dir() {
            return None;
        }

        // OpenStack style: openstack/latest/
        let openstack_latest = root.join("openstack").join("latest");
        if openstack_latest.is_dir() {
            let meta_data = Self::read_file_if_exists(&openstack_latest.join("meta_data.json"));
            let user_data = Self::read_file_if_exists(&openstack_latest.join("user_data"));
            let network_config = Self::read_file_if_exists(&openstack_latest.join("network_data.json"));
            let vendor_data = Self::read_file_if_exists(&openstack_latest.join("vendor_data.json"));

            if meta_data.is_some() || user_data.is_some() || network_config.is_some() {
                return Some(RawCloudData {
                    source_type: "ConfigDrive:OpenStack".to_string(),
                    meta_data,
                    user_data,
                    network_config,
                    vendor_data,
                });
            }
        }

        // EC2 style: ec2/latest/
        let ec2_latest = root.join("ec2").join("latest");
        if ec2_latest.is_dir() {
            let meta_data = Self::read_file_if_exists(&ec2_latest.join("meta-data"));
            let user_data = Self::read_file_if_exists(&ec2_latest.join("user-data"));
            if meta_data.is_some() || user_data.is_some() {
                return Some(RawCloudData {
                    source_type: "ConfigDrive:EC2".to_string(),
                    meta_data,
                    user_data,
                    network_config: None,
                    vendor_data: None,
                });
            }
        }

        // Fallback to top-level directory check
        Self::load_from_directory(root, "ConfigDrive:Flat")
    }

    /// Checks `/proc/cmdline` for ds=nocloud;s=... or novainit.seed=...
    pub fn discover_cmdline() -> Option<RawCloudData> {
        let cmdline = fs::read_to_string("/proc/cmdline").unwrap_or_default();
        for param in cmdline.split_whitespace() {
            if param.starts_with("novainit.seed=") {
                let path_str = &param["novainit.seed=".len()..];
                return Self::load_from_directory(Path::new(path_str), "Cmdline:novainit.seed");
            }
            if param.starts_with("ds=nocloud;s=") {
                let path_str = &param["ds=nocloud;s=".len()..];
                return Self::load_from_directory(Path::new(path_str), "Cmdline:ds=nocloud");
            }
            if param.starts_with("ci.seed=") {
                let path_str = &param["ci.seed=".len()..];
                return Self::load_from_directory(Path::new(path_str), "Cmdline:ci.seed");
            }
        }
        None
    }

    fn read_file_if_exists(p: &Path) -> Option<String> {
        if p.is_file() {
            fs::read_to_string(p).ok()
        } else {
            None
        }
    }
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_load_from_nocloud_directory() {
        let tmp = std::env::temp_dir().join("novainit_test_nocloud");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let mut f_meta = File::create(tmp.join("meta-data")).unwrap();
        writeln!(f_meta, "instance-id: test-node-01\nlocal-hostname: test-node").unwrap();

        let mut f_user = File::create(tmp.join("user-data")).unwrap();
        writeln!(f_user, "#cloud-config\nhostname: test-node\n").unwrap();

        let data = DataSourceManager::load_from_directory(&tmp, "TestNoCloud").expect("Discovered data");
        assert_eq!(data.source_type, "TestNoCloud");
        assert!(data.meta_data.unwrap().contains("instance-id: test-node-01"));
        assert!(data.user_data.unwrap().contains("#cloud-config"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_load_from_openstack_config_drive() {
        let tmp = std::env::temp_dir().join("novainit_test_openstack");
        let _ = fs::remove_dir_all(&tmp);
        let openstack_latest = tmp.join("openstack").join("latest");
        fs::create_dir_all(&openstack_latest).unwrap();

        let mut f_meta = File::create(openstack_latest.join("meta_data.json")).unwrap();
        writeln!(f_meta, r#"{{"instance_id": "openstack-vm-99", "hostname": "os-node-1"}}"#).unwrap();

        let mut f_user = File::create(openstack_latest.join("user_data")).unwrap();
        writeln!(f_user, "#cloud-config\nusers:\n  - name: cloud-admin\n").unwrap();

        let data = DataSourceManager::load_config_drive(&tmp).expect("Discovered OpenStack data");
        assert_eq!(data.source_type, "ConfigDrive:OpenStack");
        assert!(data.meta_data.unwrap().contains("openstack-vm-99"));
        assert!(data.user_data.unwrap().contains("cloud-admin"));

        let _ = fs::remove_dir_all(&tmp);
    }
}
