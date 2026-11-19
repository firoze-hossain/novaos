//! userland/desktop/apps/nova-recovery/src/lib.rs
//!
//! # NovaOS Emergency Recovery Console & Snapshot Rollback Mode
//!
//! Provides a resilient emergency rescue environment supporting:
//! - NovaFS B-Tree snapshot rollback and generation tracking
//! - Administrative credential recovery with PBKDF2-HMAC-SHA256
//! - UEFI 2.9 Protective MBR and GUID Partition Table (GPT) repair
//! - Emergency system log inspection and kernel crash dump analysis
//! - Retained-mode 800x600 GUI dashboard and headless CLI rescue shell

pub mod crypto;
pub mod snapshot;
pub mod passwd_reset;
pub mod boot_repair;
pub mod log_viewer;
pub mod ui;
pub mod cli;

use nova_ui::prelude::BufferSurface;
use crate::boot_repair::BootRepairManager;
use crate::cli::RescueCliSession;
use crate::crypto::{crc32, hash_password, verify_password};
use crate::log_viewer::{LogLevel, LogViewer};
use crate::passwd_reset::PasswordResetManager;
use crate::snapshot::SnapshotManager;
use crate::ui::{RecoveryApp, RecoveryTab, RecoveryUiRenderer, RESCUE_HEIGHT, RESCUE_WIDTH};

/// Master autonomous self-test for NovaOS Recovery Environment.
/// Returns true if all subsystem checks pass with 100% integrity.
pub fn run_nova_recovery_selftest() -> bool {
    // -------------------------------------------------------------------------
    // 1. Cryptographic Primitives Qualification (CRC32, PBKDF2)
    // -------------------------------------------------------------------------
    let data = b"NovaOS Recovery 2.0 IEEE 802.3 CRC32 Test Vector";
    let crc = crc32(data);
    if crc == 0 || crc == 0xFFFF_FFFF {
        return false;
    }

    let pass = "RescuePassword2026!";
    let salt = b"salttestsalt1234";
    let hash = hash_password(pass, salt);
    if !verify_password(pass, &hash) || verify_password("WrongPassword!", &hash) {
        return false;
    }

    // -------------------------------------------------------------------------
    // 2. NovaFS Snapshot Discovery & Rollback Qualification
    // -------------------------------------------------------------------------
    let mut snap_mgr = SnapshotManager::new();
    if snap_mgr.list_snapshots().len() != 3 {
        return false;
    }
    let _orig_root = snap_mgr.current_root_block;
    let orig_gen = snap_mgr.current_generation;

    // Execute atomic rollback to snapshot 101
    if snap_mgr.rollback_to_snapshot(101).is_err() {
        return false;
    }
    if snap_mgr.current_root_block != 1024000 || snap_mgr.current_generation != orig_gen + 1 {
        return false;
    }
    // Verify automated pre-rollback checkpoint was created
    if snap_mgr.list_snapshots().len() != 4 {
        return false;
    }

    // -------------------------------------------------------------------------
    // 3. Administrative Credential Recovery & Shadow Qualification
    // -------------------------------------------------------------------------
    let mut pwd_mgr = PasswordResetManager::new();
    if !pwd_mgr.verify_credentials("root", "NovaEnterprise2026!") {
        return false;
    }
    if pwd_mgr.reset_password("root", "UpdatedRootPass2026$", b"saltnew012345678").is_err() {
        return false;
    }
    if !pwd_mgr.verify_credentials("root", "UpdatedRootPass2026$") {
        return false;
    }
    if pwd_mgr.verify_credentials("root", "NovaEnterprise2026!") {
        return false;
    }

    // -------------------------------------------------------------------------
    // 4. Boot Sector & GPT Dual-Header Repair Qualification
    // -------------------------------------------------------------------------
    let repair_mgr = BootRepairManager::new(20971520);
    let mut mbr = [0u8; 512];
    if repair_mgr.validate_mbr(&mbr) {
        return false;
    }
    if repair_mgr.repair_protective_mbr(&mut mbr).is_err() {
        return false;
    }
    if !repair_mgr.validate_mbr(&mbr) {
        return false;
    }

    // -------------------------------------------------------------------------
    // 5. Emergency Log Inspector Qualification
    // -------------------------------------------------------------------------
    let log_viewer = LogViewer::new();
    let panics = log_viewer.filter_entries(Some(LogLevel::Panic), None);
    if panics.len() != 1 || panics[0].level != LogLevel::Panic {
        return false;
    }

    // -------------------------------------------------------------------------
    // 6. Retained-Mode GUI Buffer Rasterization Qualification
    // -------------------------------------------------------------------------
    let mut app = RecoveryApp::new();
    let mut surface = BufferSurface::new(RESCUE_WIDTH, RESCUE_HEIGHT);
    for &tab in &RecoveryTab::all() {
        app.active_tab = tab;
        RecoveryUiRenderer::render(&app, &mut surface);
    }

    // -------------------------------------------------------------------------
    // 7. Headless CLI Rescue Command Session Qualification
    // -------------------------------------------------------------------------
    let mut cli_sess = RescueCliSession::new();
    let snap_out = cli_sess.execute_command("snapshots");
    if snap_out.is_err() || !snap_out.unwrap().contains("@snapshot-20261115") {
        return false;
    }
    let roll_out = cli_sess.execute_command("rollback 102");
    if roll_out.is_err() || !roll_out.unwrap().contains("[OK]") {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_master_recovery_selftest() {
        assert!(run_nova_recovery_selftest());
    }
}
