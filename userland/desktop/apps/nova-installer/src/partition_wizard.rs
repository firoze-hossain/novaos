//! userland/desktop/apps/nova-installer/src/partition_wizard.rs
//!
//! Drive Partitioning Wizard for NovaOS Desktop Graphical Installer.
//! Supports:
//! - Automatic Guided Partitioning ("Erase entire disk and install NovaOS")
//! - Custom Manual Partitioning with GPT table manipulation
//! - UEFI ESP (FAT32), NovaFS (CoW B-Tree RootFS & Home), Swap, and Journal partitions
//! - Alignment validation to 1MB (2048 sector) boundaries
//! - Visual multi-colored partition bar gauge rendering

use std::fmt;

// Standard GPT Partition Type GUIDs (RFC 4122 / UEFI Spec)
pub const GUID_EFI_SYSTEM: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
]; // c12a7328-f81f-11d2-ba4b-00a0c93ec93b

pub const GUID_LINUX_ROOT: [u8; 16] = [
    0x0f, 0x3f, 0xc1, 0x4f, 0x60, 0xab, 0x4d, 0x4d, 0x8a, 0xae, 0x3b, 0xb8, 0x93, 0x86, 0x98, 0x69,
]; // 4f68bce3-e8cd-4db1-96e7-fbcaf984b709 (NovaOS RootFS)

pub const GUID_LINUX_DATA: [u8; 16] = [
    0xa2, 0xa0, 0xd0, 0xeb, 0xe5, 0xb9, 0x33, 0x44, 0x87, 0xc0, 0x68, 0xb6, 0xb7, 0x26, 0x99, 0xc7,
]; // ebd0a0a2-b9e5-4433-87c0-68b6b72699c7 (NovaOS Home/Data)

pub const GUID_LINUX_SWAP: [u8; 16] = [
    0x6d, 0xfd, 0x57, 0x06, 0xab, 0xa4, 0xc4, 0x43, 0x84, 0xe5, 0x09, 0x33, 0xc8, 0x4b, 0x4f, 0x4f,
]; // 0657fd6d-a4ab-43c4-84e5-0933c84b4f4f (Swap / CrashDump)

pub const SECTOR_SIZE: u64 = 512;
pub const ALIGNMENT_SECTORS: u64 = 2048; // 1 MiB alignment
pub const GPT_RESERVED_START: u64 = 2048; // First usable LBA (1 MiB)
pub const GPT_BACKUP_SECTORS: u64 = 34;   // 33 sectors for GPT + 1 sector safety

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusType {
    Nvme,
    VirtIo,
    Sata,
    Scsi,
    Usb,
}

impl fmt::Display for BusType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BusType::Nvme => write!(f, "NVMe"),
            BusType::VirtIo => write!(f, "VirtIO"),
            BusType::Sata => write!(f, "SATA"),
            BusType::Scsi => write!(f, "SCSI"),
            BusType::Usb => write!(f, "USB"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiskDevice {
    pub path: String,
    pub model: String,
    pub bus_type: BusType,
    pub size_bytes: u64,
    pub sector_size: u32,
    pub total_sectors: u64,
    pub is_removable: bool,
}

impl DiskDevice {
    pub fn new(path: &str, model: &str, bus_type: BusType, size_bytes: u64) -> Self {
        let sector_size = 512;
        let total_sectors = size_bytes / (sector_size as u64);
        Self {
            path: path.to_string(),
            model: model.to_string(),
            bus_type,
            size_bytes,
            sector_size,
            total_sectors,
            is_removable: false,
        }
    }

    pub fn size_gb(&self) -> f64 {
        self.size_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
    }

    pub fn formatted_size(&self) -> String {
        format!("{:.1} GB", self.size_gb())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionType {
    EfiSystem,
    RootFs,
    Home,
    Swap,
    Journal,
    CrashDump,
    Custom,
}

impl fmt::Display for PartitionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PartitionType::EfiSystem => write!(f, "EFI System Partition"),
            PartitionType::RootFs => write!(f, "NovaOS Root (/)"),
            PartitionType::Home => write!(f, "NovaOS Data (/home)"),
            PartitionType::Swap => write!(f, "Swap / Journal"),
            PartitionType::Journal => write!(f, "Write-Ahead Journal"),
            PartitionType::CrashDump => write!(f, "Crash Dump Partition"),
            PartitionType::Custom => write!(f, "Custom Partition"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemType {
    NovaFs,
    Fat32,
    Ext4,
    Swap,
    Unformatted,
}

impl fmt::Display for FilesystemType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilesystemType::NovaFs => write!(f, "NovaFS (CoW B-Tree)"),
            FilesystemType::Fat32 => write!(f, "FAT32"),
            FilesystemType::Ext4 => write!(f, "ext4"),
            FilesystemType::Swap => write!(f, "swap"),
            FilesystemType::Unformatted => write!(f, "Unformatted"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PartitionEntry {
    pub index: usize,
    pub name: String,
    pub part_type: PartitionType,
    pub fs_type: FilesystemType,
    pub mount_point: String,
    pub start_lba: u64,
    pub sector_count: u64,
    pub size_bytes: u64,
    pub format: bool,
    pub bootable: bool,
    pub type_guid: [u8; 16],
    pub unique_guid: [u8; 16],
}

impl PartitionEntry {
    pub fn size_gb(&self) -> f64 {
        self.size_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
    }

    pub fn formatted_size(&self) -> String {
        if self.size_bytes < 1024 * 1024 * 1024 {
            let mb = self.size_bytes as f64 / (1024.0 * 1024.0);
            format!("{:.0} MB", mb)
        } else {
            format!("{:.1} GB", self.size_gb())
        }
    }

    pub fn end_lba(&self) -> u64 {
        self.start_lba.saturating_add(self.sector_count).saturating_sub(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionMode {
    GuidedAutomatic,
    CustomManual,
}

#[derive(Debug, Clone)]
pub struct VisualPartitionSegment {
    pub name: String,
    pub fs_type: String,
    pub mount_point: String,
    pub start_pct: f32,
    pub width_pct: f32,
    pub size_label: String,
    pub color_rgb: u32,
}

pub struct PartitionWizard {
    pub available_disks: Vec<DiskDevice>,
    pub selected_disk_idx: usize,
    pub mode: PartitionMode,
    pub partitions: Vec<PartitionEntry>,
    pub next_part_index: usize,
}

impl PartitionWizard {
    pub fn new() -> Self {
        let default_disks = Self::detect_system_disks();
        let mut wizard = Self {
            available_disks: default_disks,
            selected_disk_idx: 0,
            mode: PartitionMode::GuidedAutomatic,
            partitions: Vec::new(),
            next_part_index: 1,
        };
        wizard.apply_guided_automatic();
        wizard
    }

    /// Discovers system disks or provides hardware emulations
    pub fn detect_system_disks() -> Vec<DiskDevice> {
        let mut disks = Vec::new();

        // 1. Primary NVMe drive (e.g. 100 GB enterprise cloud/desktop drive)
        disks.push(DiskDevice::new(
            "/dev/nvme0n1",
            "NovaOS NVMe 2.0 High-Performance SSD (100 GB)",
            BusType::Nvme,
            100 * 1024 * 1024 * 1024,
        ));

        // 2. VirtIO / SATA secondary drive (e.g. 500 GB storage pool)
        disks.push(DiskDevice::new(
            "/dev/vda",
            "VirtIO Modern Fast Block Storage (500 GB)",
            BusType::VirtIo,
            500 * 1024 * 1024 * 1024,
        ));

        // 3. Fallback SATA drive
        disks.push(DiskDevice::new(
            "/dev/sda",
            "SATA III Enterprise Storage Disk (250 GB)",
            BusType::Sata,
            250 * 1024 * 1024 * 1024,
        ));

        disks
    }

    pub fn selected_disk(&self) -> Option<&DiskDevice> {
        self.available_disks.get(self.selected_disk_idx)
    }

    pub fn select_disk(&mut self, idx: usize) {
        if idx < self.available_disks.len() {
            self.selected_disk_idx = idx;
            if self.mode == PartitionMode::GuidedAutomatic {
                self.apply_guided_automatic();
            }
        }
    }

    pub fn set_mode(&mut self, mode: PartitionMode) {
        self.mode = mode;
        if mode == PartitionMode::GuidedAutomatic {
            self.apply_guided_automatic();
        }
    }

    /// Generates standard production 4-partition GPT layout:
    /// - Partition 1: EFI System Partition (ESP, 512 MB, FAT32, /boot/efi)
    /// - Partition 2: NovaOS RootFS (NovaFS, ~40% of disk or min 30GB, /)
    /// - Partition 3: NovaOS Data/Home (NovaFS, remainder minus swap, /home)
    /// - Partition 4: Swap & Journal (Swap, 4096 MB)
    pub fn apply_guided_automatic(&mut self) {
        self.partitions.clear();
        let disk = match self.selected_disk() {
            Some(d) => d.clone(),
            None => return,
        };

        let total_sectors = disk.total_sectors;
        let last_usable_lba = total_sectors.saturating_sub(GPT_BACKUP_SECTORS + 1);

        let mut current_lba = GPT_RESERVED_START;

        // 1. ESP Partition: 512 MiB = 1,048,576 sectors
        let esp_sectors: u64 = (512 * 1024 * 1024) / SECTOR_SIZE;
        let p1 = PartitionEntry {
            index: 1,
            name: "EFI System Partition".to_string(),
            part_type: PartitionType::EfiSystem,
            fs_type: FilesystemType::Fat32,
            mount_point: "/boot/efi".to_string(),
            start_lba: current_lba,
            sector_count: esp_sectors,
            size_bytes: esp_sectors * SECTOR_SIZE,
            format: true,
            bootable: true,
            type_guid: GUID_EFI_SYSTEM,
            unique_guid: [0x11; 16],
        };
        current_lba += esp_sectors;
        self.partitions.push(p1);

        // 4. Swap Partition: 4096 MiB = 8,388,608 sectors (reserve at end)
        let swap_sectors: u64 = (4096 * 1024 * 1024) / SECTOR_SIZE;
        let swap_start_lba = (last_usable_lba.saturating_sub(swap_sectors)) & !(ALIGNMENT_SECTORS - 1);

        // Remaining sectors for RootFS and Home
        let remaining_sectors = swap_start_lba.saturating_sub(current_lba);

        // RootFS takes 40% (min 20GB, max 60GB)
        let mut root_sectors = (remaining_sectors * 40) / 100;
        let min_root_sectors = (20 * 1024 * 1024 * 1024u64) / SECTOR_SIZE;
        if root_sectors < min_root_sectors && remaining_sectors > min_root_sectors {
            root_sectors = min_root_sectors;
        }
        root_sectors = (root_sectors / ALIGNMENT_SECTORS) * ALIGNMENT_SECTORS;

        let p2 = PartitionEntry {
            index: 2,
            name: "NovaOS System Root (/)".to_string(),
            part_type: PartitionType::RootFs,
            fs_type: FilesystemType::NovaFs,
            mount_point: "/".to_string(),
            start_lba: current_lba,
            sector_count: root_sectors,
            size_bytes: root_sectors * SECTOR_SIZE,
            format: true,
            bootable: false,
            type_guid: GUID_LINUX_ROOT,
            unique_guid: [0x22; 16],
        };
        current_lba += root_sectors;
        self.partitions.push(p2);

        // Home partition takes the remainder up to swap_start_lba
        let home_sectors = swap_start_lba.saturating_sub(current_lba);
        let p3 = PartitionEntry {
            index: 3,
            name: "NovaOS User Data (/home)".to_string(),
            part_type: PartitionType::Home,
            fs_type: FilesystemType::NovaFs,
            mount_point: "/home".to_string(),
            start_lba: current_lba,
            sector_count: home_sectors,
            size_bytes: home_sectors * SECTOR_SIZE,
            format: true,
            bootable: false,
            type_guid: GUID_LINUX_DATA,
            unique_guid: [0x33; 16],
        };
        self.partitions.push(p3);

        // Swap partition
        let p4 = PartitionEntry {
            index: 4,
            name: "Swap & CrashDump".to_string(),
            part_type: PartitionType::Swap,
            fs_type: FilesystemType::Swap,
            mount_point: "swap".to_string(),
            start_lba: swap_start_lba,
            sector_count: swap_sectors,
            size_bytes: swap_sectors * SECTOR_SIZE,
            format: true,
            bootable: false,
            type_guid: GUID_LINUX_SWAP,
            unique_guid: [0x44; 16],
        };
        self.partitions.push(p4);

        self.next_part_index = 5;
    }

    /// Adds a manual partition with alignment check
    pub fn add_manual_partition(
        &mut self,
        name: &str,
        part_type: PartitionType,
        fs_type: FilesystemType,
        mount_point: &str,
        size_mb: u64,
    ) -> Result<(), String> {
        let disk = self.selected_disk().ok_or("No disk selected")?.clone();
        let sectors = ((size_mb * 1024 * 1024) / SECTOR_SIZE) / ALIGNMENT_SECTORS * ALIGNMENT_SECTORS;
        if sectors == 0 {
            return Err("Partition size too small".to_string());
        }

        let mut start_lba = GPT_RESERVED_START;
        for p in &self.partitions {
            if p.end_lba() >= start_lba {
                start_lba = ((p.end_lba() + 1 + ALIGNMENT_SECTORS - 1) / ALIGNMENT_SECTORS) * ALIGNMENT_SECTORS;
            }
        }

        let last_usable_lba = disk.total_sectors.saturating_sub(GPT_BACKUP_SECTORS + 1);
        if start_lba + sectors > last_usable_lba {
            return Err(format!(
                "Not enough contiguous free space on disk (requested {} sectors, available {})",
                sectors,
                last_usable_lba.saturating_sub(start_lba)
            ));
        }

        let type_guid = match part_type {
            PartitionType::EfiSystem => GUID_EFI_SYSTEM,
            PartitionType::RootFs => GUID_LINUX_ROOT,
            PartitionType::Home => GUID_LINUX_DATA,
            PartitionType::Swap | PartitionType::CrashDump => GUID_LINUX_SWAP,
            _ => GUID_LINUX_DATA,
        };

        let entry = PartitionEntry {
            index: self.next_part_index,
            name: name.to_string(),
            part_type,
            fs_type,
            mount_point: mount_point.to_string(),
            start_lba,
            sector_count: sectors,
            size_bytes: sectors * SECTOR_SIZE,
            format: true,
            bootable: part_type == PartitionType::EfiSystem,
            type_guid,
            unique_guid: [self.next_part_index as u8; 16],
        };

        self.next_part_index += 1;
        self.partitions.push(entry);
        Ok(())
    }

    /// Deletes a partition by index
    pub fn delete_partition(&mut self, index: usize) -> bool {
        if let Some(pos) = self.partitions.iter().position(|p| p.index == index) {
            self.partitions.remove(pos);
            true
        } else {
            false
        }
    }

    /// Validates current partition layout against enterprise rules
    pub fn validate_layout(&self) -> Result<(), String> {
        let disk = self.selected_disk().ok_or("No disk selected")?;

        if self.partitions.is_empty() {
            return Err("No partitions defined on target disk".to_string());
        }

        // 1. Check for EFI System Partition
        let has_esp = self.partitions.iter().any(|p| {
            p.part_type == PartitionType::EfiSystem && p.fs_type == FilesystemType::Fat32
        });
        if !has_esp {
            return Err("Missing EFI System Partition (ESP). NovaOS requires a FAT32 ESP partition for UEFI boot.".to_string());
        }

        // 2. Check for Root Partition (/)
        let has_root = self.partitions.iter().any(|p| p.mount_point == "/");
        if !has_root {
            return Err("Missing root mount point ('/'). A root partition is required to install NovaOS.".to_string());
        }

        // 3. Boundary & Overlap checks
        let last_usable_lba = disk.total_sectors.saturating_sub(GPT_BACKUP_SECTORS + 1);

        for i in 0..self.partitions.len() {
            let p1 = &self.partitions[i];

            if p1.start_lba < GPT_RESERVED_START {
                return Err(format!("Partition {} starts before first usable LBA (LBA {})", p1.name, p1.start_lba));
            }
            if p1.end_lba() > last_usable_lba {
                return Err(format!("Partition {} extends beyond last usable disk LBA ({})", p1.name, last_usable_lba));
            }

            // Alignment check
            if p1.start_lba % ALIGNMENT_SECTORS != 0 {
                return Err(format!("Partition {} start LBA {} is not 1MB aligned", p1.name, p1.start_lba));
            }

            // Overlap check
            for j in (i + 1)..self.partitions.len() {
                let p2 = &self.partitions[j];
                if p1.start_lba <= p2.end_lba() && p2.start_lba <= p1.end_lba() {
                    return Err(format!("Partitions '{}' and '{}' overlap", p1.name, p2.name));
                }
            }
        }

        Ok(())
    }

    /// Computes visual segments for rendering the partition bar gauge
    pub fn compute_visual_segments(&self) -> Vec<VisualPartitionSegment> {
        let disk = match self.selected_disk() {
            Some(d) => d,
            None => return Vec::new(),
        };

        let total = disk.total_sectors as f32;
        if total <= 0.0 {
            return Vec::new();
        }

        let mut segments = Vec::new();

        for p in &self.partitions {
            let start_pct = (p.start_lba as f32 / total) * 100.0;
            let width_pct = (p.sector_count as f32 / total) * 100.0;

            let color_rgb = match p.part_type {
                PartitionType::EfiSystem => 0x0078D4, // Nova Blue
                PartitionType::RootFs => 0x107C41,    // Emerald Green
                PartitionType::Home => 0x8764B8,      // Purple
                PartitionType::Swap | PartitionType::CrashDump => 0xD83B01, // Amber/Orange
                _ => 0x5C2D91,
            };

            segments.push(VisualPartitionSegment {
                name: p.name.clone(),
                fs_type: p.fs_type.to_string(),
                mount_point: p.mount_point.clone(),
                start_pct,
                width_pct,
                size_label: p.formatted_size(),
                color_rgb,
            });
        }

        segments
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guided_partitioning_generation_and_validation() {
        let wizard = PartitionWizard::new();
        assert_eq!(wizard.mode, PartitionMode::GuidedAutomatic);
        assert_eq!(wizard.partitions.len(), 4);

        // Verify layout passes validation
        assert!(wizard.validate_layout().is_ok());

        // Verify ESP is FAT32 and bootable
        let esp = &wizard.partitions[0];
        assert_eq!(esp.part_type, PartitionType::EfiSystem);
        assert_eq!(esp.fs_type, FilesystemType::Fat32);
        assert_eq!(esp.mount_point, "/boot/efi");
        assert!(esp.bootable);

        // Verify RootFS
        let root = &wizard.partitions[1];
        assert_eq!(root.mount_point, "/");
        assert_eq!(root.fs_type, FilesystemType::NovaFs);

        // Verify segments
        let segments = wizard.compute_visual_segments();
        assert_eq!(segments.len(), 4);
        assert!(segments[0].width_pct > 0.0);
    }

    #[test]
    fn test_custom_partitioning_validation() {
        let mut wizard = PartitionWizard::new();
        wizard.set_mode(PartitionMode::CustomManual);
        wizard.partitions.clear();

        // Should fail when empty
        assert!(wizard.validate_layout().is_err());

        // Add ESP only (missing root)
        wizard.add_manual_partition("ESP", PartitionType::EfiSystem, FilesystemType::Fat32, "/boot/efi", 512).unwrap();
        assert!(wizard.validate_layout().is_err());

        // Add Root (now valid)
        wizard.add_manual_partition("Root", PartitionType::RootFs, FilesystemType::NovaFs, "/", 30720).unwrap();
        assert!(wizard.validate_layout().is_ok());
    }
}
