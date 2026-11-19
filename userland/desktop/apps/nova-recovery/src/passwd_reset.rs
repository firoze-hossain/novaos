//! userland/desktop/apps/nova-recovery/src/passwd_reset.rs
//!
//! Administrative Credential Recovery & Root Password Reset Subsystem.
//! Directly manages `/etc/passwd` and `/etc/shadow` records with PBKDF2-HMAC-SHA256.

use crate::crypto::{hash_password, verify_password};

/// User entry parsed from `/etc/shadow`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShadowEntry {
    pub username: String,
    pub password_hash: String,
    pub last_changed_days: u32,
    pub min_days: u32,
    pub max_days: u32,
    pub warn_days: u32,
    pub inactive_days: String,
    pub expire_days: String,
    pub reserved: String,
}

impl ShadowEntry {
    pub fn parse_line(line: &str) -> Option<Self> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }
        let parts: Vec<&str> = trimmed.split(':').collect();
        if parts.len() < 2 {
            return None;
        }

        Some(Self {
            username: parts[0].to_string(),
            password_hash: parts[1].to_string(),
            last_changed_days: parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(19700),
            min_days: parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(0),
            max_days: parts.get(4).and_then(|s| s.parse().ok()).unwrap_or(99999),
            warn_days: parts.get(5).and_then(|s| s.parse().ok()).unwrap_or(7),
            inactive_days: parts.get(6).unwrap_or(&"").to_string(),
            expire_days: parts.get(7).unwrap_or(&"").to_string(),
            reserved: parts.get(8).unwrap_or(&"").to_string(),
        })
    }

    pub fn to_shadow_line(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}",
            self.username,
            self.password_hash,
            self.last_changed_days,
            self.min_days,
            self.max_days,
            self.warn_days,
            self.inactive_days,
            self.expire_days,
            self.reserved
        )
    }

    pub fn is_locked(&self) -> bool {
        self.password_hash.starts_with('!') || self.password_hash.starts_with('*')
    }
}

/// Password Reset Manager operating on shadow user database
pub struct PasswordResetManager {
    pub entries: Vec<ShadowEntry>,
}

impl PasswordResetManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            entries: Vec::new(),
        };
        mgr.seed_default_users();
        mgr
    }

    pub fn from_shadow_content(content: &str) -> Self {
        let mut entries = Vec::new();
        for line in content.lines() {
            if let Some(entry) = ShadowEntry::parse_line(line) {
                entries.push(entry);
            }
        }
        Self { entries }
    }

    /// Seeds standard root and operator accounts for testing and recovery environment
    pub fn seed_default_users(&mut self) {
        let root_hash = hash_password("NovaEnterprise2026!", b"rootsalt12345678");
        self.entries.push(ShadowEntry {
            username: "root".to_string(),
            password_hash: root_hash,
            last_changed_days: 19800,
            min_days: 0,
            max_days: 99999,
            warn_days: 7,
            inactive_days: String::new(),
            expire_days: String::new(),
            reserved: String::new(),
        });

        let admin_hash = hash_password("AdminSecurePass!", b"adminsalt1234567");
        self.entries.push(ShadowEntry {
            username: "novadmin".to_string(),
            password_hash: admin_hash,
            last_changed_days: 19800,
            min_days: 0,
            max_days: 99999,
            warn_days: 7,
            inactive_days: String::new(),
            expire_days: String::new(),
            reserved: String::new(),
        });
    }

    /// Lists usernames found in the shadow database
    pub fn list_usernames(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.username.clone()).collect()
    }

    /// Finds a user entry by username
    pub fn get_user(&self, username: &str) -> Option<&ShadowEntry> {
        self.entries.iter().find(|e| e.username == username)
    }

    /// Resets the target user's password with PBKDF2-HMAC-SHA256
    pub fn reset_password(
        &mut self,
        username: &str,
        new_password: &str,
        salt_bytes: &[u8],
    ) -> Result<String, String> {
        if new_password.len() < 6 {
            return Err("Password must be at least 6 characters long".to_string());
        }

        let new_hash = hash_password(new_password, salt_bytes);

        for entry in self.entries.iter_mut() {
            if entry.username == username {
                entry.password_hash = new_hash.clone();
                entry.last_changed_days = 19850; // Updated epoch days
                return Ok(new_hash);
            }
        }

        Err(format!("User '{}' not found in shadow registry", username))
    }

    /// Unlocks a locked user account by removing leading '!' or '*'
    pub fn unlock_account(&mut self, username: &str) -> Result<(), String> {
        for entry in self.entries.iter_mut() {
            if entry.username == username {
                if entry.is_locked() {
                    entry.password_hash = entry.password_hash.trim_start_matches(&['!', '*'][..]).to_string();
                }
                return Ok(());
            }
        }
        Err(format!("User '{}' not found", username))
    }

    /// Verifies if a user password matches
    pub fn verify_credentials(&self, username: &str, candidate_pass: &str) -> bool {
        if let Some(entry) = self.get_user(username) {
            verify_password(candidate_pass, &entry.password_hash)
        } else {
            false
        }
    }

    /// Serializes entire shadow database back to standard `/etc/shadow` file format
    pub fn export_shadow_file(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            out.push_str(&entry.to_shadow_line());
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shadow_parsing_and_serialization() {
        let raw = "root:$pbkdf2-sha256$4096$73616c74$hash:19800:0:99999:7:::\nnovadmin:!$locked:19800:0:99999:7:::\n";
        let mgr = PasswordResetManager::from_shadow_content(raw);
        assert_eq!(mgr.list_usernames(), vec!["root", "novadmin"]);
        assert!(mgr.get_user("novadmin").unwrap().is_locked());
    }

    #[test]
    fn test_root_password_reset_and_verification() {
        let mut mgr = PasswordResetManager::new();
        assert!(mgr.verify_credentials("root", "NovaEnterprise2026!"));

        // Reset password
        let res = mgr.reset_password("root", "NewRescuePassword2026#", b"resalt1234567890");
        assert!(res.is_ok());

        // Old password fails
        assert!(!mgr.verify_credentials("root", "NovaEnterprise2026!"));
        // New password succeeds
        assert!(mgr.verify_credentials("root", "NewRescuePassword2026#"));
    }
}
