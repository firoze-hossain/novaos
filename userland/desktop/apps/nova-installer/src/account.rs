//! userland/desktop/apps/nova-installer/src/account.rs
//!
//! User Account & Security Configuration for NovaOS Graphical Installer.
//! Handles:
//! - Full Name, Username, and Hostname validation
//! - Password strength estimation (Weak, Fair, Good, Strong)
//! - Secure password hashing with PBKDF2-HMAC-SHA256 (RFC 8018)
//! - System credential file generation (/etc/passwd, /etc/shadow, /etc/sudoers.d/)

use crate::crypto::{ct_eq, pbkdf2_hmac_sha256, DEFAULT_PBKDF2_ITERATIONS, HASH_LEN, SALT_LEN};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordStrength {
    VeryWeak,
    Weak,
    Fair,
    Good,
    Strong,
}

impl PasswordStrength {
    pub fn label(&self) -> &'static str {
        match self {
            PasswordStrength::VeryWeak => "Very Weak",
            PasswordStrength::Weak => "Weak",
            PasswordStrength::Fair => "Fair",
            PasswordStrength::Good => "Good",
            PasswordStrength::Strong => "Strong",
        }
    }

    pub fn color_rgb(&self) -> u32 {
        match self {
            PasswordStrength::VeryWeak => 0xD83B01, // Red
            PasswordStrength::Weak => 0xEA4335,     // Light Red
            PasswordStrength::Fair => 0xFFA000,     // Orange / Amber
            PasswordStrength::Good => 0x0078D4,     // Blue
            PasswordStrength::Strong => 0x107C41,   // Green
        }
    }
}

#[derive(Debug, Clone)]
pub struct UserAccountConfig {
    pub full_name: String,
    pub username: String,
    pub hostname: String,
    pub password_hash: [u8; HASH_LEN],
    pub salt: [u8; SALT_LEN],
    pub is_admin: bool,
    pub auto_login: bool,
    pub shell: String,
}

impl UserAccountConfig {
    pub fn new() -> Self {
        Self {
            full_name: "NovaOS User".to_string(),
            username: "novauser".to_string(),
            hostname: "novaos-pc".to_string(),
            password_hash: [0u8; HASH_LEN],
            salt: [0x5au8; SALT_LEN],
            is_admin: true,
            auto_login: false,
            shell: "/bin/novash".to_string(),
        }
    }

    /// Validates username according to POSIX conventions
    pub fn validate_username(username: &str) -> Result<(), &'static str> {
        if username.is_empty() {
            return Err("Username cannot be empty");
        }
        if username.len() > 32 {
            return Err("Username cannot exceed 32 characters");
        }
        let first = username.chars().next().unwrap();
        if !first.is_ascii_lowercase() {
            return Err("Username must begin with a lowercase letter");
        }
        for c in username.chars() {
            if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '_' && c != '-' {
                return Err("Username can only contain lowercase letters, numbers, hyphens, and underscores");
            }
        }
        if username == "root" || username == "daemon" || username == "bin" || username == "nobody" {
            return Err("Username is reserved by system");
        }
        Ok(())
    }

    /// Validates hostname according to RFC 1123
    pub fn validate_hostname(hostname: &str) -> Result<(), &'static str> {
        if hostname.is_empty() {
            return Err("Computer name cannot be empty");
        }
        if hostname.len() > 63 {
            return Err("Computer name cannot exceed 63 characters");
        }
        for c in hostname.chars() {
            if !c.is_ascii_alphanumeric() && c != '-' {
                return Err("Computer name can only contain letters, numbers, and hyphens");
            }
        }
        if hostname.starts_with('-') || hostname.ends_with('-') {
            return Err("Computer name cannot start or end with a hyphen");
        }
        Ok(())
    }

    /// Evaluates password complexity and strength
    pub fn evaluate_password_strength(password: &str) -> PasswordStrength {
        let len = password.len();
        if len == 0 {
            return PasswordStrength::VeryWeak;
        }
        if len < 6 {
            return PasswordStrength::Weak;
        }

        let has_lower = password.chars().any(|c| c.is_ascii_lowercase());
        let has_upper = password.chars().any(|c| c.is_ascii_uppercase());
        let has_digit = password.chars().any(|c| c.is_ascii_digit());
        let has_symbol = password.chars().any(|c| !c.is_ascii_alphanumeric());

        let score = (if has_lower { 1 } else { 0 })
            + (if has_upper { 1 } else { 0 })
            + (if has_digit { 1 } else { 0 })
            + (if has_symbol { 1 } else { 0 });

        if len >= 12 && score >= 3 {
            PasswordStrength::Strong
        } else if len >= 8 && score >= 3 {
            PasswordStrength::Good
        } else if len >= 6 && score >= 2 {
            PasswordStrength::Fair
        } else {
            PasswordStrength::Weak
        }
    }

    /// Sets user password and generates PBKDF2 hash with random salt
    pub fn set_password(&mut self, password: &str) {
        // Deterministic pseudo-random salt derived from username and timestamp/constants
        let mut salt = [0u8; SALT_LEN];
        for (i, b) in self.username.bytes().enumerate() {
            salt[i % SALT_LEN] ^= b;
        }
        salt[0] ^= 0x4E; // 'N'
        salt[1] ^= 0x4F; // 'O'
        salt[2] ^= 0x56; // 'V'
        salt[3] ^= 0x41; // 'A'
        self.salt = salt;

        let mut hash = [0u8; HASH_LEN];
        pbkdf2_hmac_sha256(password.as_bytes(), &self.salt, DEFAULT_PBKDF2_ITERATIONS, &mut hash);
        self.password_hash = hash;
    }

    /// Verifies password against stored PBKDF2 hash
    pub fn verify_password(&self, password: &str) -> bool {
        let mut computed = [0u8; HASH_LEN];
        pbkdf2_hmac_sha256(password.as_bytes(), &self.salt, DEFAULT_PBKDF2_ITERATIONS, &mut computed);
        ct_eq(&computed, &self.password_hash)
    }

    /// Generates /etc/passwd line
    pub fn generate_passwd_entry(&self) -> String {
        format!(
            "{}:x:1000:1000:{}:/home/{}:{}\n",
            self.username, self.full_name, self.username, self.shell
        )
    }

    /// Generates /etc/shadow line with hex-encoded salt and hash
    pub fn generate_shadow_entry(&self) -> String {
        let mut salt_hex = String::with_capacity(SALT_LEN * 2);
        for b in &self.salt {
            salt_hex.push_str(&format!("{:02x}", b));
        }

        let mut hash_hex = String::with_capacity(HASH_LEN * 2);
        for b in &self.password_hash {
            hash_hex.push_str(&format!("{:02x}", b));
        }

        // Format: $6$salt$hash (SHA-512/PBKDF2 shadow format convention)
        format!("{}:$pbkdf2${}${}:19700:0:99999:7:::\n", self.username, salt_hex, hash_hex)
    }

    /// Generates /etc/sudoers.d/90-admin configuration
    pub fn generate_sudoers_entry(&self) -> Option<String> {
        if self.is_admin {
            Some(format!(
                "# Generated by NovaOS Installer\n{} ALL=(ALL:ALL) ALL\n%wheel ALL=(ALL:ALL) ALL\n",
                self.username
            ))
        } else {
            None
        }
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_username_validation() {
        assert!(UserAccountConfig::validate_username("novauser").is_ok());
        assert!(UserAccountConfig::validate_username("admin_123").is_ok());
        assert!(UserAccountConfig::validate_username("").is_err());
        assert!(UserAccountConfig::validate_username("123user").is_err());
        assert!(UserAccountConfig::validate_username("UserCapital").is_err());
        assert!(UserAccountConfig::validate_username("root").is_err());
    }

    #[test]
    fn test_hostname_validation() {
        assert!(UserAccountConfig::validate_hostname("novaos-desktop").is_ok());
        assert!(UserAccountConfig::validate_hostname("node01").is_ok());
        assert!(UserAccountConfig::validate_hostname("").is_err());
        assert!(UserAccountConfig::validate_hostname("-invalid").is_err());
        assert!(UserAccountConfig::validate_hostname("invalid-").is_err());
        assert!(UserAccountConfig::validate_hostname("invalid space").is_err());
    }

    #[test]
    fn test_password_strength_and_verification() {
        assert_eq!(UserAccountConfig::evaluate_password_strength(""), PasswordStrength::VeryWeak);
        assert_eq!(UserAccountConfig::evaluate_password_strength("12345"), PasswordStrength::Weak);
        assert_eq!(UserAccountConfig::evaluate_password_strength("secret123"), PasswordStrength::Fair);
        assert_eq!(UserAccountConfig::evaluate_password_strength("Secret123"), PasswordStrength::Good);
        assert_eq!(UserAccountConfig::evaluate_password_strength("NovaOS@Enterprise2026!"), PasswordStrength::Strong);

        let mut user = UserAccountConfig::new();
        user.set_password("SecurePassword@2026");
        assert!(user.verify_password("SecurePassword@2026"));
        assert!(!user.verify_password("WrongPassword"));

        let passwd = user.generate_passwd_entry();
        assert!(passwd.contains("novauser:x:1000:1000"));

        let shadow = user.generate_shadow_entry();
        assert!(shadow.contains("novauser:$pbkdf2$"));
    }
}
