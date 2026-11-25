//! userland/desktop/apps/nova-disks/src/lib.rs
//!
//! # NovaOS Disks & Storage Studio (nova-disks / Stratum)
//!
//! Sovereign, pure-Rust storage device topology, proportional volume layout,
//! partition manager, SMART health telemetry, and benchmark suite for NovaOS,
//! synthesizing Ubuntu Disks (gnome-disk-utility) into a high-performance vector studio.

pub use stratum_core::{
    ActivePopup, BenchmarkMetrics, DiskImageManager, DiskImageRecord, DriveType,
    FilesystemType, PartitionTableType, SmartAssessment, SmartAttribute, SmartDiagnostics,
    SmartStatus, StorageDrive, StratumStudio, VolumePartition,
};
pub use stratum_render::{PixelBuffer, StratumRenderer};
pub use stratum_storage::StratumStorage;
pub use stratum_ui::{StratumUiController, UiEvent};

/// Autonomous qualification validation suite for nova-disks.
pub fn run_nova_disks_selftest() -> bool {
    let mut controller = StratumUiController::new(800, 600, "/tmp/stratum-selftest");

    // 1. Storage Drives Enumeration
    if controller.studio.drives.len() != 3 {
        return false;
    }
    if controller.studio.selected_drive().device_node != "/dev/sda" {
        return false;
    }

    // 2. Volume Partition Map
    let sda = controller.studio.selected_drive();
    if sda.partitions.len() != 4 {
        return false;
    }
    if sda.partitions[1].name != "Filesystem" || sda.partitions[1].fs_type != FilesystemType::Ext4 {
        return false;
    }

    // 3. Mount & Unmount Lifecycle
    controller.dispatch(UiEvent::SelectPartition(1));
    if !controller.studio.selected_drive().selected_partition().unwrap().is_mounted() {
        return false;
    }
    controller.dispatch(UiEvent::ToggleMount);
    if controller.studio.selected_drive().selected_partition().unwrap().is_mounted() {
        return false;
    }
    controller.dispatch(UiEvent::ToggleMount);
    if !controller.studio.selected_drive().selected_partition().unwrap().is_mounted() {
        return false;
    }

    // 4. Volume Format & Labeling
    controller.dispatch(UiEvent::SelectPartition(2));
    controller.dispatch(UiEvent::FormatPartition {
        fs_type: FilesystemType::Ext4,
        label: "NovaWorkspace".to_string(),
    });
    if controller.studio.selected_drive().selected_partition().unwrap().name != "NovaWorkspace" {
        return false;
    }

    // 5. S.M.A.R.T. Health Telemetry
    if controller.studio.smart.temperature_c != 34 {
        return false;
    }
    let test_status = controller.studio.smart.run_short_self_test();
    if !test_status.contains("No bad sectors") {
        return false;
    }

    // 6. Transfer Rate & Latency Benchmark
    controller.dispatch(UiEvent::RunBenchmark);
    if controller.studio.benchmark.read_avg_mbps < 400.0 {
        return false;
    }

    // 7. Virtual Disk Image & Loopback Tools
    controller.dispatch(UiEvent::AttachImage("/tmp/test.iso".to_string()));
    if !controller.studio.image_mgr.images.iter().any(|i| i.path == "/tmp/test.iso") {
        return false;
    }
    controller.dispatch(UiEvent::DetachImage("/tmp/test.iso".to_string()));

    // 8. Static /etc/fstab Export
    let fstab = controller.storage.generate_fstab(&controller.studio);
    if !fstab.contains("UUID=") {
        return false;
    }

    // 9. Optical Drive Selection
    controller.dispatch(UiEvent::SelectDrive(1));
    if controller.studio.selected_drive().drive_type != DriveType::Optical {
        return false;
    }

    // 10. Pixel Buffer Rendering
    controller.dispatch(UiEvent::SelectDrive(0));
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
    fn test_nova_disks_selftest() {
        assert!(run_nova_disks_selftest());
    }
}
