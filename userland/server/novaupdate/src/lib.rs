//! userland/server/novaupdate/src/lib.rs
//!
//! NovaOS 2.0 (Month 5 - Enterprise Storage): Automated Unattended Security Updates with Atomic A/B Rollback
//!
//! Provides the complete server-grade automated update engine:
//!   - Dual boot slot (Slot A / Slot B) partition manager
//!   - Cryptographically signed OS delta package verification (HMAC-SHA256 & SHA-256)
//!   - Hardware watchdog timer integration (`/dev/watchdog`)
//!   - Remote HTTPS update discovery & manifest parser
//!   - End-to-end update orchestrator with atomic commit and failsafe rollback

pub mod ab_boot;
pub mod client;
pub mod orchestrator;
pub mod package;
pub mod watchdog;

pub use ab_boot::{BootManager, BootSlotMetadata, Slot, SlotState};
pub use client::{CheckResult, RemoteUpdateInfo, UpdateClient, UpdateUrgency};
pub use orchestrator::{UpdateConfig, UpdateOrchestrator, UpdateStatusReport};
pub use package::{DeltaPackage, PackageManifest, DEFAULT_SIGNING_KEY};
pub use watchdog::{WatchdogController, WatchdogStatus};

/// Comprehensive end-to-end self-test verifying atomic A/B updates,
/// cryptographic signature validation, hardware watchdog protection, and
/// failsafe automatic rollback.
pub fn run_novaupdate_selftest() -> bool {
    println!("============================================================");
    println!("    NovaOS Automated Unattended Update Engine Self-Test     ");
    println!("============================================================");

    let sandbox_dir = std::env::temp_dir().join(format!("novaupdate_selftest_{}", std::process::id()));
    let boot_dir = sandbox_dir.join("boot");
    let watchdog_file = sandbox_dir.join("watchdog.state");
    let _ = std::fs::create_dir_all(&boot_dir);

    // 1. Dual Boot Slot Metadata
    print!("[TEST 1/8] Dual Boot Slot Metadata (A/B Partitioning)... ");
    let mut meta = BootSlotMetadata::new_default();
    meta.slot_a_version = "2.0.0".to_string();
    meta.slot_b_version = "2.0.1".to_string();
    meta.active_slot = Slot::SlotA;
    meta.fallback_slot = Slot::SlotA;
    meta.slot_state = SlotState::Normal;

    let serialized = meta.serialize();
    let deserialized = match BootSlotMetadata::deserialize(&serialized) {
        Ok(m) => m,
        Err(e) => {
            println!("FAILED: {}", e);
            let _ = std::fs::remove_dir_all(&sandbox_dir);
            return false;
        }
    };
    assert_eq!(deserialized.active_slot, Slot::SlotA);
    assert_eq!(deserialized.slot_state, SlotState::Normal);
    assert_eq!(deserialized.slot_a_version, "2.0.0");
    assert_eq!(deserialized.slot_b_version, "2.0.1");
    println!("PASSED");

    // 2. Cryptographic Package Signing & Tamper Rejection
    print!("[TEST 2/8] Cryptographic Delta Package Verification (HMAC-SHA256)... ");
    let mock_kernel_payload = vec![0x90u8; 1024]; // 1 KiB mock kernel binary
    let pkg = DeltaPackage::build(
        "novaos-sec-patch-01",
        "2.0.1",
        "2.0.0",
        "kernel",
        mock_kernel_payload.clone(),
        DEFAULT_SIGNING_KEY,
    );
    let pkg_bytes = pkg.encode();

    // Valid decode
    let decoded_pkg = match DeltaPackage::decode(&pkg_bytes, DEFAULT_SIGNING_KEY) {
        Ok(p) => p,
        Err(e) => {
            println!("FAILED: {}", e);
            let _ = std::fs::remove_dir_all(&sandbox_dir);
            return false;
        }
    };
    assert_eq!(decoded_pkg.manifest.version, "2.0.1");
    assert_eq!(decoded_pkg.payload, mock_kernel_payload);

    // Tampered payload must be rejected
    let mut tampered_bytes = pkg_bytes.clone();
    let last = tampered_bytes.len() - 1;
    tampered_bytes[last] ^= 0xFF;
    assert!(DeltaPackage::decode(&tampered_bytes, DEFAULT_SIGNING_KEY).is_err());

    // Wrong signing key must be rejected
    assert!(DeltaPackage::decode(&pkg_bytes, b"Wrong-Untrusted-Key-1234").is_err());
    println!("PASSED");

    // 3. Hardware Watchdog Integration
    print!("[TEST 3/8] Hardware Watchdog Timer (arm, ping, disarm)... ");
    let mut wdt = WatchdogController::with_custom_device(watchdog_file.clone());
    assert!(!wdt.is_armed());
    assert!(wdt.arm(45).is_ok());
    assert!(wdt.is_armed());
    assert_eq!(wdt.timeout_secs(), 45);
    assert!(wdt.ping().is_ok());
    assert!(wdt.disarm().is_ok());
    assert!(!wdt.is_armed());
    println!("PASSED");

    // 4. Staging into Inactive Boot Slot (Slot B)
    print!("[TEST 4/8] Staging Kernel Payload into Inactive Slot B... ");
    let mut orch = UpdateOrchestrator::with_sandbox(boot_dir.clone(), watchdog_file.clone());
    let initial_status = orch.get_status().unwrap();
    assert_eq!(initial_status.active_slot, Slot::SlotA);

    let pkg_file = sandbox_dir.join("update_2.0.1.pkg");
    assert!(pkg.save_to_file(&pkg_file).is_ok());

    let (staged_slot, staged_version) = match orch.stage_package(pkg_file.to_str().unwrap()) {
        Ok(res) => res,
        Err(e) => {
            println!("FAILED: {}", e);
            let _ = std::fs::remove_dir_all(&sandbox_dir);
            return false;
        }
    };
    assert_eq!(staged_slot, Slot::SlotB);
    assert_eq!(staged_version, "2.0.1");

    let post_stage_status = orch.get_status().unwrap();
    assert_eq!(post_stage_status.slot_b_version, "2.0.1");
    assert!(boot_dir.join("novaos_b.bin").exists());
    println!("PASSED");

    // 5. Arming Testing Mode & Watchdog Countdown
    print!("[TEST 5/8] Arming Testing Mode & Watchdog Countdown (3 attempts)... ");
    let test_meta = match orch.prepare_reboot(Slot::SlotB) {
        Ok(m) => m,
        Err(e) => {
            println!("FAILED: {}", e);
            let _ = std::fs::remove_dir_all(&sandbox_dir);
            return false;
        }
    };
    assert_eq!(test_meta.active_slot, Slot::SlotB);
    assert_eq!(test_meta.fallback_slot, Slot::SlotA);
    assert_eq!(test_meta.slot_state, SlotState::Testing);
    assert_eq!(test_meta.boot_attempts, 3);
    assert!(orch.watchdog.is_armed());
    println!("PASSED");

    // 6. Post-Boot Health Verification & Permanent Commit
    print!("[TEST 6/8] Post-Boot Health Verification & Atomic Commit... ");
    assert!(orch.run_health_checks().is_ok());
    let confirmed_meta = match orch.confirm_healthy() {
        Ok(m) => m,
        Err(e) => {
            println!("FAILED: {}", e);
            let _ = std::fs::remove_dir_all(&sandbox_dir);
            return false;
        }
    };
    assert_eq!(confirmed_meta.active_slot, Slot::SlotB);
    assert_eq!(confirmed_meta.fallback_slot, Slot::SlotB);
    assert_eq!(confirmed_meta.slot_state, SlotState::Normal);
    assert!(!orch.watchdog.is_armed());
    println!("PASSED");

    // 7. Failsafe Boot Attempts Countdown & Automatic Fallback
    print!("[TEST 7/8] Failsafe Boot Countdown Exhaustion & Automatic Rollback... ");
    // Arm Slot A for testing
    let _ = orch.prepare_reboot(Slot::SlotA);
    assert_eq!(orch.boot_mgr.load_metadata().unwrap().boot_attempts, 3);

    // Boot attempt 1
    let (cur_slot, rolled_back) = orch.boot_mgr.record_boot_attempt().unwrap();
    assert_eq!(cur_slot, Slot::SlotA);
    assert!(!rolled_back);
    assert_eq!(orch.boot_mgr.load_metadata().unwrap().boot_attempts, 2);

    // Boot attempt 2
    let (cur_slot, rolled_back) = orch.boot_mgr.record_boot_attempt().unwrap();
    assert_eq!(cur_slot, Slot::SlotA);
    assert!(!rolled_back);
    assert_eq!(orch.boot_mgr.load_metadata().unwrap().boot_attempts, 1);

    // Boot attempt 3: Countdown exhausted! Automatic failsafe rollback to Slot B
    let (cur_slot, rolled_back) = orch.boot_mgr.record_boot_attempt().unwrap();
    assert_eq!(cur_slot, Slot::SlotB);
    assert!(rolled_back);
    let final_meta = orch.boot_mgr.load_metadata().unwrap();
    assert_eq!(final_meta.active_slot, Slot::SlotB);
    assert_eq!(final_meta.slot_state, SlotState::RolledBack);
    println!("PASSED (Slot A failed 3 times -> automatically rolled back to Slot B)");

    // 8. Hardware Watchdog Panic Trip Simulation
    print!("[TEST 8/8] Watchdog Kernel Panic Trip & Instant Slot Fallback... ");
    let _ = orch.prepare_reboot(Slot::SlotA);
    let panic_meta = match orch.handle_boot_failure("Simulated kernel null dereference at 0x80104000") {
        Ok(m) => m,
        Err(e) => {
            println!("FAILED: {}", e);
            let _ = std::fs::remove_dir_all(&sandbox_dir);
            return false;
        }
    };
    assert_eq!(panic_meta.active_slot, Slot::SlotB);
    assert_eq!(panic_meta.slot_state, SlotState::RolledBack);
    assert!(panic_meta.last_rollback_reason.unwrap().contains("null dereference"));
    assert!(!orch.watchdog.is_armed());
    println!("PASSED");

    // Clean up sandbox
    let _ = std::fs::remove_dir_all(&sandbox_dir);

    println!("============================================================");
    println!("  ALL 8/8 ATOMIC A/B UPDATE & WATCHDOG ROLLBACK CHECKS PASSED ");
    println!("============================================================");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_novaupdate_suite() {
        assert!(run_novaupdate_selftest());
    }
}
