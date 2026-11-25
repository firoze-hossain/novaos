//! userland/desktop/apps/nova-updates/src/lib.rs
//!
//! # NovaOS Software Sources & Updates Studio (nova-updates / Nexus)
//!
//! Sovereign, pure-Rust system repository configuration, mirror latency benchmark,
//! cryptographic trust store, and software update engine for NovaOS,
//! synthesizing Ubuntu Software & Updates (7 tabs) with Software Updater (interactive modal).

pub use nexus_core::{
    AuthKeyStore, ChannelManager, CheckFrequency, DeviceCategory, DeviceDriver,
    DeveloperOptions, DownloadMirror, DriverManager, FeedManager, HardwareDevice,
    KeyAlgorithm, MirrorManager, MirrorProtocol, MirrorStatus, NexusActiveTab,
    NexusStudio, NexusWindowMode, OtherUpdatePolicy, PackageCategory, PendingPackage,
    PpaFeed, ReleaseUpgradeNotify, RepoChannel, SecurityUpdatePolicy, SigningKey,
    SoftwareUpdaterEngine, UpdatePolicy, UpdateStage,
};
pub use nexus_render::{NexusRenderer, PixelBuffer};
pub use nexus_storage::StorageManager;
pub use nexus_ui::{NexusUiController, UiEvent};

/// Autonomous qualification validation suite for nova-updates.
pub fn run_nova_updates_selftest() -> bool {
    let mut controller = NexusUiController::new(800, 600, "/tmp/nexus-selftest");

    // 1. Initial State
    if controller.studio.channels.channels.len() != 5 {
        return false;
    }

    // 2. Channel Toggling
    controller.dispatch(UiEvent::ToggleChannel("multiverse".to_string()));
    if !controller.studio.channels.is_channel_enabled("multiverse") {
        return false;
    }

    // 3. Mirror Speed Benchmark
    controller.dispatch(UiEvent::RunBenchmark);
    let selected = controller.studio.mirrors.get_selected_mirror();
    if selected.latency_ms.is_none() {
        return false;
    }

    // 4. PPA / Feed Addition and Export
    let test_feed = "deb https://packages.microsoft.com/repos/code stable main";
    let feed_id = match controller.studio.feeds.add_feed_from_line(test_feed) {
        Ok(id) => id,
        Err(_) => return false,
    };
    if !controller.studio.feeds.feeds.iter().any(|f| f.id == feed_id) {
        return false;
    }
    let exported = controller.studio.feeds.export_all_sources();
    if !exported.contains("microsoft") {
        return false;
    }

    // 5. Update Policy Configuration
    controller.dispatch(UiEvent::SetCheckFrequency(CheckFrequency::Daily));
    if controller.studio.policy.check_frequency != CheckFrequency::Daily {
        return false;
    }
    controller.dispatch(UiEvent::SetSecurityPolicy(SecurityUpdatePolicy::DownloadAndInstallAuto));
    if controller.studio.policy.security_updates != SecurityUpdatePolicy::DownloadAndInstallAuto {
        return false;
    }

    // 6. Cryptographic Trust Store
    let dummy_key = "-----BEGIN PGP PUBLIC KEY BLOCK-----\nComment: NovaOS Staging Key\nmQGNBF9Z...\n-----END PGP PUBLIC KEY BLOCK-----";
    let imported_id = match controller.studio.auth_keys.import_key_file(dummy_key) {
        Ok(id) => id,
        Err(_) => return false,
    };
    if !controller.studio.auth_keys.keys.iter().any(|k| k.key_id == imported_id) {
        return false;
    }
    controller.dispatch(UiEvent::RemoveSelectedKey);

    // 7. Additional Hardware Drivers
    controller.dispatch(UiEvent::SelectDriver {
        device_id: "pci-0000-01-00-0".to_string(),
        driver_id: "nouveau-open".to_string(),
    });
    controller.dispatch(UiEvent::ApplyDriverChanges);
    let dev = match controller.studio.drivers.devices.iter().find(|d| d.id == "pci-0000-01-00-0") {
        Some(d) => d,
        None => return false,
    };
    if dev.get_active_driver().map(|d| d.id.as_str()) != Some("nouveau-open") {
        return false;
    }

    // 8. Developer Options
    controller.dispatch(UiEvent::ToggleProposedChannel);
    if !controller.studio.developer.proposed_channel_enabled {
        return false;
    }

    // 9. Software Updater Engine & Payload
    controller.dispatch(UiEvent::SwitchMode(NexusWindowMode::SoftwareUpdater));
    if controller.studio.mode != NexusWindowMode::SoftwareUpdater {
        return false;
    }
    let payload = controller.studio.updater.total_download_display();
    if !payload.contains("MB will be downloaded") {
        return false;
    }

    // 10. Multi-Stage Update Transaction Pipeline
    while !matches!(controller.studio.updater.current_stage, UpdateStage::Completed { .. }) {
        controller.dispatch(UiEvent::StepUpdateTransaction);
    }
    match controller.studio.updater.current_stage {
        UpdateStage::Completed { packages_updated, .. } => {
            if packages_updated == 0 {
                return false;
            }
        }
        _ => return false,
    }

    // 11. Pixel Buffer Rendering
    controller.renderer.render(&controller.studio);
    if controller.renderer.buffer.pixels.len() != 800 * 600 {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nova_updates_selftest() {
        assert!(run_nova_updates_selftest());
    }
}
