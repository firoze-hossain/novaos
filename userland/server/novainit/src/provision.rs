//! userland/server/novainit/src/provision.rs
//!
//! System Provisioning Actor: manages user accounts, SSH authorized keys,
//! hostname, network configuration files, and service orchestration.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct UserSpec {
    pub name: String,
    pub gecos: Option<String>,
    pub shell: String,
    pub sudo: Option<String>,
    pub groups: Vec<String>,
    pub ssh_authorized_keys: Vec<String>,
    pub lock_passwd: bool,
    pub passwd_hash: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct WriteFileSpec {
    pub path: String,
    pub content: String,
    pub permissions: String,
    pub owner: Option<String>,
}

pub struct SystemProvisioner {
    pub root_dir: PathBuf,
}

impl Default for SystemProvisioner {
    fn default() -> Self {
        Self {
            root_dir: PathBuf::from("/"),
        }
    }
}

impl SystemProvisioner {
    pub fn new(root: PathBuf) -> Self {
        Self { root_dir: root }
    }

    fn target_path(&self, p: &str) -> PathBuf {
        let stripped = p.strip_prefix('/').unwrap_or(p);
        self.root_dir.join(stripped)
    }

    /// Sets system hostname in `/etc/hostname` and `/etc/hosts`
    pub fn apply_hostname(&self, hostname: &str, manage_hosts: bool) -> Result<(), String> {
        let hostname_clean = hostname.trim();
        if hostname_clean.is_empty() {
            return Ok(());
        }

        // 1. Write /etc/hostname
        let h_path = self.target_path("/etc/hostname");
        if let Some(parent) = h_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(&h_path, format!("{}\n", hostname_clean))
            .map_err(|e| format!("Failed to write /etc/hostname: {}", e))?;

        // 2. Set live kernel hostname if root_dir is /
        if self.root_dir == Path::new("/") {
            let _ = Command::new("hostname").arg(hostname_clean).status();
            let _ = fs::write("/proc/sys/kernel/hostname", hostname_clean);
        }

        // 3. Update /etc/hosts
        if manage_hosts {
            let hosts_path = self.target_path("/etc/hosts");
            let existing = fs::read_to_string(&hosts_path).unwrap_or_else(|_| {
                "127.0.0.1 localhost\n::1 localhost ip6-localhost ip6-loopback\n".to_string()
            });

            // Filter out existing 127.0.1.1 entries
            let lines: Vec<&str> = existing
                .lines()
                .filter(|l| !l.trim_start().starts_with("127.0.1.1"))
                .collect();

            let new_entry = format!("127.0.1.1\t{}", hostname_clean);
            let mut combined = lines.join("\n");
            combined.push('\n');
            combined.push_str(&new_entry);
            combined.push('\n');

            let _ = fs::write(&hosts_path, combined);
        }

        Ok(())
    }

    /// Provisions root authorized SSH keys in `/root/.ssh/authorized_keys`
    pub fn apply_root_ssh_keys(&self, keys: &[String]) -> Result<(), String> {
        if keys.is_empty() {
            return Ok(());
        }

        let ssh_dir = self.target_path("/root/.ssh");
        let _ = fs::create_dir_all(&ssh_dir);

        let auth_keys_path = ssh_dir.join("authorized_keys");
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&auth_keys_path)
            .map_err(|e| format!("Cannot open /root/.ssh/authorized_keys: {}", e))?;

        for key in keys {
            let clean = key.trim();
            if !clean.is_empty() {
                writeln!(f, "{}", clean).map_err(|e| format!("Write failed: {}", e))?;
            }
        }

        // Enforce permissions: directory 0700, file 0600 on Linux
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&ssh_dir, fs::Permissions::from_mode(0o700));
            let _ = fs::set_permissions(&auth_keys_path, fs::Permissions::from_mode(0o600));
        }

        Ok(())
    }

    /// Provisions a user account with home directory and SSH keys
    pub fn provision_user(&self, user: &UserSpec) -> Result<(), String> {
        let username = user.name.trim();
        if username.is_empty() {
            return Ok(());
        }

        if username == "root" {
            return self.apply_root_ssh_keys(&user.ssh_authorized_keys);
        }

        let home_dir = if username == "root" {
            self.target_path("/root")
        } else {
            self.target_path(&format!("/home/{}", username))
        };
        let _ = fs::create_dir_all(&home_dir);

        // SSH keys in user home
        if !user.ssh_authorized_keys.is_empty() {
            let ssh_dir = home_dir.join(".ssh");
            let _ = fs::create_dir_all(&ssh_dir);

            let auth_keys_path = ssh_dir.join("authorized_keys");
            let mut f = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&auth_keys_path)
                .map_err(|e| format!("Cannot open user authorized_keys: {}", e))?;

            for key in &user.ssh_authorized_keys {
                let clean = key.trim();
                if !clean.is_empty() {
                    writeln!(f, "{}", clean).map_err(|e| format!("Write failed: {}", e))?;
                }
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&ssh_dir, fs::Permissions::from_mode(0o700));
                let _ = fs::set_permissions(&auth_keys_path, fs::Permissions::from_mode(0o600));
            }
        }

        // Add user entry to /etc/passwd if not already present
        let passwd_path = self.target_path("/etc/passwd");
        if let Some(parent) = passwd_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let mut passwd_content = fs::read_to_string(&passwd_path).unwrap_or_else(|_| {
            "root:x:0:0:root:/root:/bin/novash\n".to_string()
        });

        if !passwd_content.lines().any(|l| l.starts_with(&format!("{}:", username))) {
            let uid = 1000;
            let gid = 1000;
            let gecos = user.gecos.as_deref().unwrap_or(username);
            let shell = if user.shell.is_empty() { "/bin/novash" } else { &user.shell };
            let entry = format!("{}:x:{}:{}:{}:/home/{}:{}\n", username, uid, gid, gecos, username, shell);
            passwd_content.push_str(&entry);
            fs::write(&passwd_path, passwd_content)
                .map_err(|e| format!("Failed to write /etc/passwd: {}", e))?;
        }

        // Sudo configuration in /etc/sudoers.d/
        if let Some(ref sudo_rule) = user.sudo {
            let sudoers_d = self.target_path("/etc/sudoers.d");
            let _ = fs::create_dir_all(&sudoers_d);
            let rule_path = sudoers_d.join(format!("90-cloud-init-{}", username));
            let rule_line = format!("{} {}\n", username, sudo_rule);
            let _ = fs::write(&rule_path, rule_line);

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&rule_path, fs::Permissions::from_mode(0o440));
            }
        }

        Ok(())
    }

    /// Writes files declared in `write_files` block
    pub fn write_file(&self, spec: &WriteFileSpec) -> Result<(), String> {
        let path = self.target_path(&spec.path);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        fs::write(&path, &spec.content)
            .map_err(|e| format!("Failed to write {}: {}", spec.path, e))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if !spec.permissions.is_empty() {
                let perm_clean = spec.permissions.trim_matches(['"', '\'']);
                if let Ok(mode) = u32::from_str_radix(perm_clean, 8) {
                    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(mode));
                }
            }
        }

        Ok(())
    }

    /// Executes `runcmd` commands sequentially
    pub fn execute_runcmd(&self, cmd: &[String]) -> Result<i32, String> {
        if cmd.is_empty() {
            return Ok(0);
        }

        let prog = &cmd[0];
        let args = &cmd[1..];

        let mut child = Command::new(prog);
        child.args(args);

        match child.status() {
            Ok(status) => Ok(status.code().unwrap_or(0)),
            Err(e) => Err(format!("Command '{}' failed: {}", prog, e)),
        }
    }
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provision_hostname_and_hosts() {
        let tmp = std::env::temp_dir().join("novainit_test_prov_host");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let prov = SystemProvisioner::new(tmp.clone());
        prov.apply_hostname("novacloud-node1", true).unwrap();

        let h_content = fs::read_to_string(tmp.join("etc/hostname")).unwrap();
        assert_eq!(h_content.trim(), "novacloud-node1");

        let hosts = fs::read_to_string(tmp.join("etc/hosts")).unwrap();
        assert!(hosts.contains("127.0.1.1\tnovacloud-node1"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_provision_user_and_ssh_keys() {
        let tmp = std::env::temp_dir().join("novainit_test_prov_user");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let prov = SystemProvisioner::new(tmp.clone());
        let user = UserSpec {
            name: "novaadmin".into(),
            gecos: Some("Cloud Administrator".into()),
            shell: "/bin/novash".into(),
            sudo: Some("ALL=(ALL) NOPASSWD:ALL".into()),
            groups: vec!["wheel".into(), "sudo".into()],
            ssh_authorized_keys: vec![
                "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIInw2BkW1+8n83fM5R admin@novaos".into(),
            ],
            lock_passwd: true,
            passwd_hash: None,
        };

        prov.provision_user(&user).unwrap();

        let auth_keys = fs::read_to_string(tmp.join("home/novaadmin/.ssh/authorized_keys")).unwrap();
        assert!(auth_keys.contains("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5"));

        let passwd = fs::read_to_string(tmp.join("etc/passwd")).unwrap();
        assert!(passwd.contains("novaadmin:x:1000:1000:Cloud Administrator:/home/novaadmin:/bin/novash"));

        let sudo_rule = fs::read_to_string(tmp.join("etc/sudoers.d/90-cloud-init-novaadmin")).unwrap();
        assert!(sudo_rule.contains("novaadmin ALL=(ALL) NOPASSWD:ALL"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_provision_write_files() {
        let tmp = std::env::temp_dir().join("novainit_test_prov_files");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let prov = SystemProvisioner::new(tmp.clone());
        let spec = WriteFileSpec {
            path: "/etc/motd".into(),
            content: "Welcome to NovaOS Server 2.0!\n".into(),
            permissions: "0644".into(),
            owner: Some("root:root".into()),
        };

        prov.write_file(&spec).unwrap();
        let content = fs::read_to_string(tmp.join("etc/motd")).unwrap();
        assert_eq!(content, "Welcome to NovaOS Server 2.0!\n");

        let _ = fs::remove_dir_all(&tmp);
    }
}
