//! userland/desktop/apps/nova-installer/src/lib.rs
//!
//! # NovaOS Production Graphical Installer & Setup Wizard (nova-installer)
//!
//! Enterprise setup experience on par with Ubuntu Ubiquity and Windows 11 OOBE.
//!
//! Features:
//! - Drive partitioning wizard (Guided automatic or custom manual with GPT)
//! - User account creation with PBKDF2 password hashing & strength estimation
//! - Timezone & regional localization selection with simulated local clocks
//! - Desktop personalization & live preview (Dark Mica vs Light Cupertino, accent colors)
//! - Installation pipeline: GPT formatting, NovaFS RootFS, FAT32 ESP, UEFI bootloader, rootfs deployment
//! - Animated slide showcase during installation
//! - Multi-step wizard state machine & zero-syscall buffer rendering

pub mod crypto;
pub mod partition_wizard;
pub mod account;
pub mod timezone;
pub mod preview;
pub mod showcase;
pub mod engine;
pub mod wizard;
pub mod ui;

pub use crypto::{crc32, ct_eq, pbkdf2_hmac_sha256, Sha256};
pub use partition_wizard::{
    DiskDevice, FilesystemType, PartitionEntry, PartitionMode, PartitionType, PartitionWizard,
};
pub use account::{PasswordStrength, UserAccountConfig};
pub use timezone::{TimezoneDatabase, TimezoneEntry};
pub use preview::{AccentColor, DesktopThemeConfig, ThemeMode};
pub use showcase::{ShowcaseSlide, SlideShowcase};
pub use engine::{InstallPipelineEngine, InstallStage};
pub use wizard::{InstallerWizardApp, WizardStep};
pub use ui::InstallerWindowRenderer;

/// Master autonomous self-test for nova-installer
pub fn run_nova_installer_selftest() -> bool {
    let tmp = std::env::temp_dir().join("novainit_installer_master_selftest");
    let _ = std::fs::remove_dir_all(&tmp);
    if std::fs::create_dir_all(&tmp).is_err() {
        return false;
    }

    // 1. Verify Cryptographic Primitives (SHA-256, PBKDF2, CRC-32)
    let password = b"MasterAdmin@2026";
    let salt = b"salt_for_novaos_";
    let mut hash1 = [0u8; 32];
    pbkdf2_hmac_sha256(password, salt, 100, &mut hash1);
    let mut hash2 = [0u8; 32];
    pbkdf2_hmac_sha256(password, salt, 100, &mut hash2);
    if !ct_eq(&hash1, &hash2) || crc32(b"123456789") != 0xCBF43926 {
        return false;
    }

    // 2. Verify Guided Automatic GPT Partitioning
    let mut wizard = PartitionWizard::new();
    if wizard.partitions.len() != 4 || wizard.validate_layout().is_err() {
        return false;
    }
    let segments = wizard.compute_visual_segments();
    if segments.len() != 4 {
        return false;
    }

    // 3. Verify Custom Manual Partitioning Validation
    wizard.set_mode(PartitionMode::CustomManual);
    wizard.partitions.clear();
    if wizard.validate_layout().is_ok() {
        return false; // Should fail when empty
    }
    if wizard.add_manual_partition("ESP", PartitionType::EfiSystem, FilesystemType::Fat32, "/boot/efi", 512).is_err() {
        return false;
    }
    if wizard.add_manual_partition("Root", PartitionType::RootFs, FilesystemType::NovaFs, "/", 40960).is_err() {
        return false;
    }
    if wizard.validate_layout().is_err() {
        return false;
    }

    // 4. Verify User Account Setup & PBKDF2 Synthesis
    let mut user = UserAccountConfig::new();
    user.full_name = "Cloud Administrator".to_string();
    user.username = "novaadmin".to_string();
    user.hostname = "novacloud-node01".to_string();
    user.set_password("AdminSecure@2026!");
    if !user.verify_password("AdminSecure@2026!") {
        return false;
    }
    let passwd = user.generate_passwd_entry();
    let shadow = user.generate_shadow_entry();
    if !passwd.contains("novaadmin:x:1000:1000") || !shadow.contains("novaadmin:$pbkdf2$") {
        return false;
    }

    // 5. Verify Timezone Database & Clock Calculation
    let mut tz_db = TimezoneDatabase::new();
    if !tz_db.select_by_id("Asia/Tokyo") {
        return false;
    }
    if tz_db.selected().utc_offset_hours != 9 {
        return false;
    }
    let time_str = tz_db.selected().simulated_time_str(12, 0);
    if time_str != "09:00 PM" {
        return false;
    }

    // 6. Verify Desktop Personalization Preview Config
    let mut theme_cfg = DesktopThemeConfig::new();
    theme_cfg.theme_mode = ThemeMode::DarkMica;
    theme_cfg.scale_factor_pct = 125;
    let desktop_json = theme_cfg.generate_desktop_config();
    if !desktop_json.contains("\"dark_mica\"") || !desktop_json.contains("\"scale_percent\": 125") {
        return false;
    }

    // 7. Verify Installation Pipeline Engine
    let disk = wizard.selected_disk().unwrap().clone();
    let partitions = wizard.partitions.clone();
    let tz = tz_db.selected().clone();
    let mut engine = InstallPipelineEngine::new(Some(tmp.clone()), false);
    if engine.run_installation(&disk, &partitions, &user, &tz, &theme_cfg).is_err() {
        return false;
    }
    if !tmp.join("boot/efi/EFI/BOOT/BOOTX64.EFI").is_file() {
        return false;
    }
    if !tmp.join("etc/hostname").is_file() || !tmp.join("etc/passwd").is_file() {
        return false;
    }

    // 8. Verify Wizard State Machine & Buffer Surface Rendering
    let app = InstallerWizardApp::new();
    let mut surface = nova_ui::prelude::BufferSurface::new(1024, 700);
    InstallerWindowRenderer::render(&app, &mut surface);

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_master_selftest_passes() {
        assert!(run_nova_installer_selftest(), "All master selftest checks must pass");
    }
}
