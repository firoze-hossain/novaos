//! userland/server/novainit/src/cloud_init.rs
//!
//! NovaOS Cloud-Init Automated Provisioning Engine.
//! Coordinates multi-provider datasource discovery, YAML/JSON parsing,
//! network configuration, user account creation, SSH key injection, and service startup.

use std::fs;
use std::path::PathBuf;

use crate::datasource::DataSourceManager;
use crate::network::ParsedNetworkConfig;
use crate::parser::{JsonParser, YamlParser};
use crate::provision::{SystemProvisioner, UserSpec, WriteFileSpec};

#[derive(Debug, Clone, Default)]
pub struct CloudConfig {
    pub instance_id: String,
    pub hostname: String,
    pub manage_etc_hosts: bool,
    pub users: Vec<UserSpec>,
    pub write_files: Vec<WriteFileSpec>,
    pub runcmd: Vec<Vec<String>>,
    pub network: Option<ParsedNetworkConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudInitStatus {
    NotStarted,
    Running,
    Done,
    Disabled,
    Error(String),
}

pub struct CloudInitEngine {
    pub root_dir: PathBuf,
    pub seed_dir: Option<String>,
    pub config: CloudConfig,
    pub status: CloudInitStatus,
}

impl Default for CloudInitEngine {
    fn default() -> Self {
        Self::new(PathBuf::from("/"), None)
    }
}

impl CloudInitEngine {
    pub fn new(root_dir: PathBuf, seed_dir: Option<String>) -> Self {
        Self {
            root_dir,
            seed_dir,
            config: CloudConfig::default(),
            status: CloudInitStatus::NotStarted,
        }
    }

    fn state_dir(&self) -> PathBuf {
        self.root_dir.join("var/lib/cloud")
    }

    fn instance_dir(&self) -> PathBuf {
        self.state_dir().join("instance")
    }

    fn status_file(&self) -> PathBuf {
        self.root_dir.join("run/cloud-init/status.json")
    }

    /// Checks if this instance has already been fully provisioned (idempotency check)
    pub fn is_already_provisioned(&self, instance_id: &str) -> bool {
        let prev_id_file = self.instance_dir().join("instance-id");
        if prev_id_file.is_file() {
            if let Ok(content) = fs::read_to_string(&prev_id_file) {
                if content.trim() == instance_id.trim() && !instance_id.trim().is_empty() {
                    return true;
                }
            }
        }
        false
    }

    /// Loads and parses raw cloud data from detected datasource
    pub fn load_and_parse(&mut self) -> Result<bool, String> {
        let raw = match DataSourceManager::discover(&self.root_dir, self.seed_dir.as_deref()) {
            Some(r) => r,
            None => {
                self.status = CloudInitStatus::Disabled;
                return Ok(false);
            }
        };

        // 1. Parse meta-data
        if let Some(ref meta) = raw.meta_data {
            if meta.trim_start().starts_with('{') {
                if let Ok(json) = JsonParser::parse(meta) {
                    if let Some(id) = json.get_str("instance_id").or_else(|| json.get_str("instance-id")) {
                        self.config.instance_id = id.to_string();
                    }
                    if let Some(h) = json.get_str("hostname").or_else(|| json.get_str("local-hostname")) {
                        self.config.hostname = h.to_string();
                    }
                }
            } else if let Ok(yaml) = YamlParser::parse(meta) {
                if let Some(id) = yaml.get_str("instance-id").or_else(|| yaml.get_str("instance_id")) {
                    self.config.instance_id = id.to_string();
                }
                if let Some(h) = yaml.get_str("local-hostname").or_else(|| yaml.get_str("hostname")) {
                    self.config.hostname = h.to_string();
                }
            }
        }

        // Fallback default instance-id if none specified
        if self.config.instance_id.is_empty() {
            self.config.instance_id = "novacloud-default".to_string();
        }

        // 2. Parse user-data (#cloud-config)
        if let Some(ref user) = raw.user_data {
            if let Ok(yaml) = YamlParser::parse(user) {
                if let Some(h) = yaml.get_str("hostname") {
                    self.config.hostname = h.to_string();
                }
                if let Some(m) = yaml.get_bool("manage_etc_hosts") {
                    self.config.manage_etc_hosts = m;
                }

                // Parse users
                if let Some(users_list) = yaml.get("users").and_then(|u| u.as_list()) {
                    for u in users_list {
                        if let Some(name) = u.get_str("name") {
                            let mut spec = UserSpec {
                                name: name.to_string(),
                                gecos: u.get_str("gecos").map(|s| s.to_string()),
                                shell: u.get_str("shell").unwrap_or("/bin/novash").to_string(),
                                sudo: u.get_str("sudo").map(|s| s.to_string()),
                                groups: Vec::new(),
                                ssh_authorized_keys: Vec::new(),
                                lock_passwd: u.get_bool("lock_passwd").unwrap_or(true),
                                passwd_hash: u.get_str("passwd").map(|s| s.to_string()),
                            };

                            if let Some(groups) = u.get("groups").and_then(|g| g.as_list()) {
                                for g in groups {
                                    if let Some(gs) = g.as_str() {
                                        spec.groups.push(gs.to_string());
                                    }
                                }
                            }

                            if let Some(keys) = u.get("ssh_authorized_keys").and_then(|k| k.as_list()) {
                                for k in keys {
                                    if let Some(ks) = k.as_str() {
                                        spec.ssh_authorized_keys.push(ks.to_string());
                                    }
                                }
                            }

                            self.config.users.push(spec);
                        }
                    }
                }

                // Parse write_files
                if let Some(files) = yaml.get("write_files").and_then(|f| f.as_list()) {
                    for f in files {
                        if let Some(path) = f.get_str("path") {
                            let spec = WriteFileSpec {
                                path: path.to_string(),
                                content: f.get_str("content").unwrap_or("").to_string(),
                                permissions: f.get_str("permissions").unwrap_or("0644").to_string(),
                                owner: f.get_str("owner").map(|s| s.to_string()),
                            };
                            self.config.write_files.push(spec);
                        }
                    }
                }

                // Parse runcmd
                if let Some(cmds) = yaml.get("runcmd").and_then(|c| c.as_list()) {
                    for cmd in cmds {
                        if let Some(cmd_list) = cmd.as_list() {
                            let mut vec = Vec::new();
                            for item in cmd_list {
                                if let Some(s) = item.as_str() {
                                    vec.push(s.to_string());
                                }
                            }
                            if !vec.is_empty() {
                                self.config.runcmd.push(vec);
                            }
                        } else if let Some(cmd_str) = cmd.as_str() {
                            let parts: Vec<String> = cmd_str.split_whitespace().map(|s| s.to_string()).collect();
                            if !parts.is_empty() {
                                self.config.runcmd.push(parts);
                            }
                        }
                    }
                }
            }
        }

        // 3. Parse network-config
        if let Some(ref net) = raw.network_config {
            if let Ok(yaml) = YamlParser::parse(net) {
                if let Ok(parsed_net) = ParsedNetworkConfig::parse_netplan_v2(&yaml) {
                    self.config.network = Some(parsed_net);
                }
            }
        }

        Ok(true)
    }

    /// Stage 1: `init` (Local)
    /// Discovers datasource, writes hostname and network interface configuration.
    pub fn run_stage_local(&mut self) -> Result<(), String> {
        let provisioner = SystemProvisioner::new(self.root_dir.clone());

        // Apply hostname
        if !self.config.hostname.is_empty() {
            provisioner.apply_hostname(&self.config.hostname, self.config.manage_etc_hosts)?;
        }

        // Apply network configuration
        if let Some(ref net) = self.config.network {
            let ifaces_text = net.generate_interfaces_file();
            provisioner.write_file(&WriteFileSpec {
                path: "/etc/network/interfaces".into(),
                content: ifaces_text,
                permissions: "0644".into(),
                owner: None,
            })?;

            let resolv_text = net.generate_resolv_conf();
            provisioner.write_file(&WriteFileSpec {
                path: "/etc/resolv.conf".into(),
                content: resolv_text,
                permissions: "0644".into(),
                owner: None,
            })?;
        }

        Ok(())
    }

    /// Stage 2: `config`
    /// Provisions users, SSH authorized keys, and writes declared configuration files.
    pub fn run_stage_config(&mut self) -> Result<(), String> {
        let provisioner = SystemProvisioner::new(self.root_dir.clone());

        // Provision user accounts and SSH keys
        for user in &self.config.users {
            provisioner.provision_user(user)?;
        }

        // Write files
        for f in &self.config.write_files {
            provisioner.write_file(f)?;
        }

        Ok(())
    }

    /// Stage 3: `final`
    /// Executes `runcmd` commands and writes completion semaphores.
    pub fn run_stage_final(&mut self) -> Result<(), String> {
        let provisioner = SystemProvisioner::new(self.root_dir.clone());

        // Run post-boot commands
        for cmd in &self.config.runcmd {
            let _ = provisioner.execute_runcmd(cmd);
        }

        // Write instance tracking files
        let inst_dir = self.instance_dir();
        let _ = fs::create_dir_all(&inst_dir);
        let _ = fs::write(inst_dir.join("instance-id"), &self.config.instance_id);
        let _ = fs::write(inst_dir.join("boot-finished"), "1\n");

        // Write status JSON
        let status_p = self.status_file();
        if let Some(parent) = status_p.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let status_json = format!(
            "{{\n  \"status\": \"done\",\n  \"instance_id\": \"{}\",\n  \"hostname\": \"{}\",\n  \"users_count\": {}\n}}\n",
            self.config.instance_id,
            self.config.hostname,
            self.config.users.len()
        );
        let _ = fs::write(&status_p, status_json);

        self.status = CloudInitStatus::Done;
        Ok(())
    }

    /// Executes complete provisioning lifecycle
    pub fn run_all(&mut self, force: bool) -> Result<CloudInitStatus, String> {
        self.status = CloudInitStatus::Running;

        let discovered = self.load_and_parse()?;
        if !discovered {
            return Ok(CloudInitStatus::Disabled);
        }

        if !force && self.is_already_provisioned(&self.config.instance_id) {
            self.status = CloudInitStatus::Done;
            return Ok(CloudInitStatus::Done);
        }

        self.run_stage_local()?;
        self.run_stage_config()?;
        self.run_stage_final()?;

        self.status = CloudInitStatus::Done;
        Ok(CloudInitStatus::Done)
    }
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cloud_init_end_to_end_lifecycle() {
        let tmp = std::env::temp_dir().join("novainit_test_e2e");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // Prepare test seed directory
        let seed = tmp.join("var/lib/cloud/seed/nocloud");
        fs::create_dir_all(&seed).unwrap();

        let meta = r#"
instance-id: novanode-qemu-01
local-hostname: novanode-qemu
cloud-name: novaos-enterprise
"#;
        fs::write(seed.join("meta-data"), meta).unwrap();

        let user_data = r#"#cloud-config
hostname: novanode-qemu
manage_etc_hosts: true
users:
  - name: novaadmin
    gecos: Enterprise Administrator
    shell: /bin/novash
    sudo: ALL=(ALL) NOPASSWD:ALL
    groups: [wheel, sudo]
    ssh_authorized_keys:
      - ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIInw2BkW1+8n83fM5R admin@novaos
write_files:
  - path: /etc/motd
    content: "Welcome to NovaOS Cloud Server!\n"
    permissions: "0644"
runcmd:
  - [ echo, "cloud-init provisioned" ]
"#;
        fs::write(seed.join("user-data"), user_data).unwrap();

        let net_config = r#"
version: 2
ethernets:
  eth0:
    dhcp4: true
    dhcp6: false
"#;
        fs::write(seed.join("network-config"), net_config).unwrap();

        let mut engine = CloudInitEngine::new(tmp.clone(), None);
        let res = engine.run_all(false).expect("Run cloud-init");
        assert_eq!(res, CloudInitStatus::Done);

        // Verify hostname and hosts
        let h = fs::read_to_string(tmp.join("etc/hostname")).unwrap();
        assert_eq!(h.trim(), "novanode-qemu");

        let hosts = fs::read_to_string(tmp.join("etc/hosts")).unwrap();
        assert!(hosts.contains("127.0.1.1\tnovanode-qemu"));

        // Verify user and authorized keys
        let auth_keys = fs::read_to_string(tmp.join("home/novaadmin/.ssh/authorized_keys")).unwrap();
        assert!(auth_keys.contains("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5"));

        // Verify write_files
        let motd = fs::read_to_string(tmp.join("etc/motd")).unwrap();
        assert_eq!(motd, "Welcome to NovaOS Cloud Server!\n");

        // Verify instance tracking and status
        let inst_id = fs::read_to_string(tmp.join("var/lib/cloud/instance/instance-id")).unwrap();
        assert_eq!(inst_id.trim(), "novanode-qemu-01");

        let status = fs::read_to_string(tmp.join("run/cloud-init/status.json")).unwrap();
        assert!(status.contains("\"status\": \"done\""));

        // Test idempotency: running again should immediately return Done without changes
        let mut engine2 = CloudInitEngine::new(tmp.clone(), None);
        let res2 = engine2.run_all(false).expect("Second run (idempotent)");
        assert_eq!(res2, CloudInitStatus::Done);

        let _ = fs::remove_dir_all(&tmp);
    }
}
