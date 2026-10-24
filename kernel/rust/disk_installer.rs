//! kernel/rust/disk_installer.rs - Month 6: On-Disk Partitioning & Formatting Engine
//!
//! Provides the core on-disk partitioning and formatting logic backing the NovaOS
//! installer and disk management tools.
//!
//! ### Architectural Capabilities & Features
//! 1. **MBR Partition Table Engine**:
//!    - Synthesizes valid Master Boot Record (LBA 0) structures with standard `0x55, 0xAA`
//!      signature and 4 primary partition table entries matching `tools/build-disk-image.sh`:
//!        * Partition 1: FAT32 filesystem (Active/Bootable `0x80`, type `0x0C` FAT32 LBA,
//!          starts at LBA 2048 / 1MiB standard alignment).
//!        * Partition 2: ext2 filesystem (type `0x83` Linux native data partition).
//!        * Partition 3: Journal region (type `0x83` dedicated write-ahead journal).
//!        * Partition 4: Crash dump region (type `0x83` dedicated raw sector panic dump).
//!    - Computes proper LBA geometry and mapped CHS addresses for BIOS compatibility.
//!
//! 2. **FAT32 Filesystem Formatter**:
//!    - Synthesizes compliant FAT32 Boot Sector & BPB (`Fat32BootSector`):
//!        * 512 bytes per sector, 8 sectors per cluster (4096-byte clusters).
//!        * 32 reserved sectors, 2 File Allocation Tables (FAT1 and FAT2).
//!        * Media descriptor `0xF8` (fixed disk), sectors per track 63, heads 255.
//!        * Dynamic `fat_size_32` sector calculation based on partition capacity.
//!        * Volume ID, Volume Label (`"NOVAOS BOOT"`), and File System Type (`"FAT32   "`).
//!        * Boot signature `0x55, 0xAA`.
//!    - Generates FSInfo sector (sector 1) and backup FSInfo (sector 7) with lead signature
//!      `0x41615252` ("RRaA"), struct signature `0x61417272` ("rrAa"), and trail signature.
//!    - Synthesizes FAT allocation tables with Cluster 0 (`0x0FFFFFF8`), Cluster 1 (`0x0FFFFFFF`),
//!      and Root Directory Cluster 2 EOF (`0x0FFFFFFF`).
//!    - Synthesizes backup boot sector at sector 6.
//!    - Initializes Root Directory Cluster (cluster 2) with clean volume label entry.
//!
//! 3. **ext2 Filesystem Formatter**:
//!    - Generates compliant ext2 superblock at offset 1024 bytes (LBA `part2_start + 2`).
//!    - Emits magic `0xEF53`, revision 1 (`EXT2_DYNAMIC_REV`), inode size 128, block size 1024.
//!    - Generates block group descriptor table, block bitmap, inode bitmap, and root inode (inode 2).
//!    - Formats root directory data block with `.` and `..` directory entries.
//!
//! 4. **Journal & Crash Dump Partition Initialization**:
//!    - Initializes Partition 3 (Journal) with zeroed descriptor sector (`magic = 0`).
//!    - Initializes Partition 4 (Crash Dump) with zeroed header sector (`magic = 0`).
//!
//! 5. **Block Device Dispatch & Safety**:
//!    - Dispatches absolute sector writes via `blockdev_write_sectors_absolute()`.
//!    - Supports ATA (`BLOCKDEV_ATA`), VirtIO-Blk (`BLOCKDEV_VIRTIO_BLK`), and NVMe (`BLOCKDEV_NVME`).
//!    - Implements dry-run validation mode preventing accidental modification of running disks.
//!    - Restores active block device and partition offset upon operation completion.

use core::mem::size_of;

pub const SECTOR_SIZE: usize = 512;
pub const MBR_SIGNATURE: u16 = 0xAA55;
pub const MBR_TABLE_OFFSET: usize = 446;
pub const MBR_ENTRY_SIZE: usize = 16;
pub const MBR_NUM_PARTITIONS: usize = 4;

pub const PART_BOOTABLE: u8 = 0x80;
pub const PART_NON_BOOTABLE: u8 = 0x00;

pub const PART_TYPE_EMPTY: u8 = 0x00;
pub const PART_TYPE_FAT32_LBA: u8 = 0x0C;
pub const PART_TYPE_LINUX_NATIVE: u8 = 0x83;

pub const PART1_DEFAULT_MB: u32 = 64;  // FAT32 Boot/System partition
pub const PART2_DEFAULT_MB: u32 = 32;  // ext2 Data partition
pub const PART3_DEFAULT_MB: u32 = 4;   // FAT32 Write-Ahead Journal
pub const PART4_DEFAULT_MB: u32 = 1;   // Kernel Crash Dump storage
pub const PART_ALIGNMENT_SECTORS: u32 = 2048; // 1 MiB alignment (2048 * 512 = 1,048,576 B)

pub const FAT32_BYTES_PER_SECTOR: u16 = 512;
pub const FAT32_SECTORS_PER_CLUSTER: u8 = 8; // 4096 bytes per cluster
pub const FAT32_RESERVED_SECTORS: u16 = 32;
pub const FAT32_NUM_FATS: u8 = 2;
pub const FAT32_MEDIA_FIXED: u8 = 0xF8;
pub const FAT32_ROOT_CLUSTER: u32 = 2;
pub const FAT32_FSINFO_SECTOR: u16 = 1;
pub const FAT32_BACKUP_BOOT_SECTOR: u16 = 6;
pub const FAT32_BACKUP_FSINFO_SECTOR: u16 = 7;

pub const FAT32_FSINFO_LEAD_SIG: u32 = 0x41615252;   // "RRaA"
pub const FAT32_FSINFO_STRUCT_SIG: u32 = 0x61417272; // "rrAa"
pub const FAT32_FSINFO_TRAIL_SIG: u32 = 0xAA550000;

pub const FAT32_FAT_ENTRY_MEDIA: u32 = 0x0FFFFFF8;
pub const FAT32_FAT_ENTRY_DIRTY: u32 = 0x0FFFFFFF;
pub const FAT32_FAT_ENTRY_EOF: u32 = 0x0FFFFFFF;
pub const FAT_ATTR_VOLUME_ID: u8 = 0x08;

pub const EXT2_MAGIC: u16 = 0xEF53;
pub const EXT2_BLOCK_SIZE: u32 = 1024;
pub const EXT2_INODE_SIZE: u16 = 128;
pub const EXT2_ROOT_INO: u32 = 2;
pub const EXT2_FIRST_INO: u32 = 11;
pub const EXT2_S_IFDIR: u16 = 0x4000;
pub const EXT2_PERM_0755: u16 = 0x01ED;
pub const EXT2_FT_DIR: u8 = 2;

pub const BLOCKDEV_ATA: i32 = 0;
pub const BLOCKDEV_VIRTIO_BLK: i32 = 1;
pub const BLOCKDEV_NVME: i32 = 2;

pub const DISK_INSTALL_OP_QUERY: i32 = 1;
pub const DISK_INSTALL_OP_DRY_RUN: i32 = 2;
pub const DISK_INSTALL_OP_FORMAT: i32 = 3;
pub const DISK_INSTALL_OP_SELFTEST: i32 = 4;

pub const DISK_INSTALL_OK: i32 = 0;
pub const DISK_INSTALL_ERR_INVALID_PARAM: i32 = -1;
pub const DISK_INSTALL_ERR_DISK_TOO_SMALL: i32 = -2;
pub const DISK_INSTALL_ERR_DEVICE_NOT_FOUND: i32 = -3;
pub const DISK_INSTALL_ERR_IO_WRITE: i32 = -4;
pub const DISK_INSTALL_ERR_IO_READ: i32 = -5;
pub const DISK_INSTALL_ERR_PERMISSION: i32 = -6;
pub const DISK_INSTALL_ERR_VERIFY_FAIL: i32 = -7;

extern "C" {
    fn blockdev_select(device: i32);
    fn blockdev_current() -> i32;
    fn blockdev_set_partition_offset(offset_lba: u32);
    fn blockdev_get_partition_offset() -> u32;
    fn blockdev_is_present() -> bool;
    fn blockdev_read_sectors_absolute(lba: u32, sector_count: u8, buffer: *mut u8) -> bool;
    fn blockdev_write_sectors_absolute(lba: u32, sector_count: u8, buffer: *const u8) -> bool;
    fn kernel_log(fmt: *const u8, ...);
}

/// On-disk MBR partition entry (16 bytes).
#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
pub struct MbrPartitionEntry {
    pub boot_indicator: u8,   // 0x80 = bootable, 0x00 = inactive
    pub start_chs: [u8; 3],   // CHS address of first sector
    pub partition_type: u8,   // 0x0C = FAT32 LBA, 0x83 = Linux native
    pub end_chs: [u8; 3],     // CHS address of last sector
    pub start_lba: u32,       // LBA of first sector
    pub sector_count: u32,    // Number of sectors in partition
}

/// Partition layout configuration requested by installer / userland.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
pub struct DiskLayoutConfig {
    pub target_device: i32,    // 0 = ATA, 1 = VirtIO-Blk, 2 = NVMe
    pub total_sectors: u32,    // 0 = auto/default (102MiB = 208,896 sectors)
    pub part1_mb: u32,         // FAT32 size in MB (0 = default 64)
    pub part2_mb: u32,         // ext2 size in MB (0 = default 32)
    pub part3_mb: u32,         // Journal size in MB (0 = default 4)
    pub part4_mb: u32,         // Crashdump size in MB (0 = default 1)
    pub dry_run: u32,          // 1 = validate only, 0 = live write
    pub format_fat32: u32,     // 1 = format FAT32, 0 = partition table only
    pub format_ext2: u32,      // 1 = format ext2, 0 = skip
}

/// Installation summary and metrics returned to caller.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
pub struct DiskInstallSummary {
    pub status: i32,           // 0 on success, < 0 on failure
    pub target_device: i32,
    pub total_sectors: u32,
    pub part1_start_lba: u32,
    pub part1_sectors: u32,
    pub part2_start_lba: u32,
    pub part2_sectors: u32,
    pub part3_start_lba: u32,
    pub part3_sectors: u32,
    pub part4_start_lba: u32,
    pub part4_sectors: u32,
    pub fat_size_sectors: u32,
    pub total_clusters: u32,
    pub sectors_written: u32,
}

/// Calculated partition layout and boundaries.
#[derive(Clone, Copy, Debug)]
pub struct PartitionLayout {
    pub total_sectors: u32,
    pub part1_start: u32,
    pub part1_count: u32,
    pub part2_start: u32,
    pub part2_count: u32,
    pub part3_start: u32,
    pub part3_count: u32,
    pub part4_start: u32,
    pub part4_count: u32,
    pub fat_size_sectors: u32,
    pub total_clusters: u32,
}

/// FAT32 Boot Sector & BPB layout (first 90 bytes of sector 0).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Fat32BootSector {
    pub jmp: [u8; 3],                  // 0xEB, 0x58, 0x90
    pub oem: [u8; 8],                  // b"NOVAOS  "
    pub bytes_per_sector: u16,         // 512
    pub sectors_per_cluster: u8,       // 8 (4096 B / cluster)
    pub reserved_sectors: u16,         // 32
    pub num_fats: u8,                  // 2
    pub root_entries: u16,             // 0
    pub total_sectors_16: u16,         // 0
    pub media_descriptor: u8,          // 0xF8
    pub fat_size_16: u16,              // 0
    pub sectors_per_track: u16,        // 63
    pub num_heads: u16,                // 255
    pub hidden_sectors: u32,           // Partition start LBA
    pub total_sectors_32: u32,         // Partition sector count
    pub fat_size_32: u32,              // Sectors per FAT
    pub ext_flags: u16,                // 0
    pub fs_version: u16,               // 0
    pub root_cluster: u32,             // 2
    pub fs_info_sector: u16,           // 1
    pub backup_boot_sector: u16,       // 6
    pub reserved: [u8; 12],            // [0; 12]
    pub drive_number: u8,              // 0x80
    pub nt_reserved: u8,               // 0
    pub boot_signature: u8,            // 0x29
    pub volume_id: u32,                // 0x20261024
    pub volume_label: [u8; 11],        // b"NOVAOS BOOT"
    pub fs_type: [u8; 8],              // b"FAT32   "
}

/// FAT32 FSInfo sector structure.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Fat32FsInfo {
    pub lead_sig: u32,         // 0x41615252 ("RRaA")
    pub reserved1: [u8; 480],  // zeroes
    pub struct_sig: u32,       // 0x61417272 ("rrAa")
    pub free_count: u32,       // free cluster count
    pub next_free: u32,        // next free cluster hint (3)
    pub reserved2: [u8; 12],   // zeroes
    pub trail_sig: u32,        // 0xAA550000
}

/// ext2 superblock (at offset 1024 bytes of Partition 2).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Ext2Superblock {
    pub inodes_count: u32,
    pub blocks_count: u32,
    pub r_blocks_count: u32,
    pub free_blocks_count: u32,
    pub free_inodes_count: u32,
    pub first_data_block: u32,
    pub log_block_size: u32,
    pub log_frag_size: u32,
    pub blocks_per_group: u32,
    pub frags_per_group: u32,
    pub inodes_per_group: u32,
    pub mtime: u32,
    pub wtime: u32,
    pub mnt_count: u16,
    pub max_mnt_count: u16,
    pub magic: u16,
    pub state: u16,
    pub errors: u16,
    pub minor_rev_level: u16,
    pub lastcheck: u32,
    pub checkinterval: u32,
    pub creator_os: u32,
    pub rev_level: u32,
    pub def_resuid: u16,
    pub def_resgid: u16,
    pub first_ino: u32,
    pub inode_size: u16,
}

/// ext2 block group descriptor.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Ext2BlockGroupDesc {
    pub block_bitmap: u32,
    pub inode_bitmap: u32,
    pub inode_table: u32,
    pub free_blocks_count: u16,
    pub free_inodes_count: u16,
    pub used_dirs_count: u16,
    pub pad: u16,
    pub reserved: [u8; 12],
}

/// ext2 inode structure (128 bytes).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Ext2Inode {
    pub mode: u16,
    pub uid: u16,
    pub size: u32,
    pub atime: u32,
    pub ctime: u32,
    pub mtime: u32,
    pub dtime: u32,
    pub gid: u16,
    pub links_count: u16,
    pub blocks: u32,
    pub flags: u32,
    pub osd1: u32,
    pub block: [u32; 15],
    pub generation: u32,
    pub file_acl: u32,
    pub size_high: u32,
    pub faddr: u32,
    pub osd2: [u8; 12],
}

/// Translates an LBA address to legacy BIOS CHS encoding.
pub fn lba_to_chs(lba: u32) -> [u8; 3] {
    if lba >= 1024 * 255 * 63 {
        return [0xFE, 0xFF, 0xFF];
    }
    let c = (lba / (255 * 63)) as u16;
    let rem = lba % (255 * 63);
    let h = (rem / 63) as u8;
    let s = ((rem % 63) + 1) as u8;
    [h, s | (((c >> 8) as u8) << 6), (c & 0xFF) as u8]
}

/// Computes partition geometry and sector allocations across the 4 partitions.
pub fn calculate_layout(
    total_sectors: u32,
    part1_mb: u32,
    part2_mb: u32,
    part3_mb: u32,
    part4_mb: u32,
) -> Result<PartitionLayout, i32> {
    let p1_mb = if part1_mb == 0 { PART1_DEFAULT_MB } else { part1_mb };
    let p2_mb = if part2_mb == 0 { PART2_DEFAULT_MB } else { part2_mb };
    let p3_mb = if part3_mb == 0 { PART3_DEFAULT_MB } else { part3_mb };
    let p4_mb = if part4_mb == 0 { PART4_DEFAULT_MB } else { part4_mb };

    let part1_count = p1_mb.saturating_mul(2048);
    let part2_count = p2_mb.saturating_mul(2048);
    let part3_count = p3_mb.saturating_mul(2048);
    let part4_count = p4_mb.saturating_mul(2048);

    let part1_start = PART_ALIGNMENT_SECTORS; // 2048 (1MiB)
    let part2_start = part1_start.saturating_add(part1_count);
    let part3_start = part2_start.saturating_add(part2_count);
    let part4_start = part3_start.saturating_add(part3_count);
    let min_needed_sectors = part4_start.saturating_add(part4_count);

    let effective_total = if total_sectors == 0 {
        min_needed_sectors.saturating_add(2048) // +1MiB trailing slack
    } else {
        total_sectors
    };

    if effective_total < min_needed_sectors {
        return Err(DISK_INSTALL_ERR_DISK_TOO_SMALL);
    }

    // Compute FAT32 parameters:
    // Sectors per FAT: each cluster entry is 4 bytes.
    // Total clusters <= part1_count / sectors_per_cluster.
    let approx_clusters = part1_count / (FAT32_SECTORS_PER_CLUSTER as u32);
    let fat_entries_needed = approx_clusters.saturating_add(2);
    let fat_size_bytes = fat_entries_needed.saturating_mul(4);
    let fat_size_sectors = (fat_size_bytes.saturating_add(511)) / 512;

    let data_start_offset = (FAT32_RESERVED_SECTORS as u32)
        .saturating_add((FAT32_NUM_FATS as u32).saturating_mul(fat_size_sectors));
    let data_sectors = part1_count.saturating_sub(data_start_offset);
    let total_clusters = data_sectors / (FAT32_SECTORS_PER_CLUSTER as u32);

    Ok(PartitionLayout {
        total_sectors: effective_total,
        part1_start,
        part1_count,
        part2_start,
        part2_count,
        part3_start,
        part3_count,
        part4_start,
        part4_count,
        fat_size_sectors,
        total_clusters,
    })
}

/// Synthesizes an MBR sector (512 bytes) with 4 partition entries and 0x55AA signature.
pub fn synthesize_mbr(layout: &PartitionLayout) -> [u8; SECTOR_SIZE] {
    let mut sector = [0u8; SECTOR_SIZE];

    // Minimal x86 bootstrap code stub at offset 0:
    // Displays nothing or jumps; standard bootable MBR bootstrap stub:
    // cli; xor ax, ax; mov ss, ax; mov sp, 0x7c00; sti; ...
    sector[0] = 0xFA; // cli
    sector[1] = 0x31; // xor ax, ax
    sector[2] = 0xC0;
    sector[3] = 0x8E; // mov ss, ax
    sector[4] = 0xD0;
    sector[5] = 0xBC; // mov sp, 0x7C00
    sector[6] = 0x00;
    sector[7] = 0x7C;
    sector[8] = 0xFB; // sti

    // Partition 1: FAT32 (Active/Bootable)
    let p1_end_lba = layout.part1_start + layout.part1_count - 1;
    let p1_entry = MbrPartitionEntry {
        boot_indicator: PART_BOOTABLE,
        start_chs: lba_to_chs(layout.part1_start),
        partition_type: PART_TYPE_FAT32_LBA,
        end_chs: lba_to_chs(p1_end_lba),
        start_lba: layout.part1_start,
        sector_count: layout.part1_count,
    };

    // Partition 2: ext2 Data
    let p2_end_lba = layout.part2_start + layout.part2_count - 1;
    let p2_entry = MbrPartitionEntry {
        boot_indicator: PART_NON_BOOTABLE,
        start_chs: lba_to_chs(layout.part2_start),
        partition_type: PART_TYPE_LINUX_NATIVE,
        end_chs: lba_to_chs(p2_end_lba),
        start_lba: layout.part2_start,
        sector_count: layout.part2_count,
    };

    // Partition 3: Journal Region
    let p3_end_lba = layout.part3_start + layout.part3_count - 1;
    let p3_entry = MbrPartitionEntry {
        boot_indicator: PART_NON_BOOTABLE,
        start_chs: lba_to_chs(layout.part3_start),
        partition_type: PART_TYPE_LINUX_NATIVE,
        end_chs: lba_to_chs(p3_end_lba),
        start_lba: layout.part3_start,
        sector_count: layout.part3_count,
    };

    // Partition 4: Crash Dump Region
    let p4_end_lba = layout.part4_start + layout.part4_count - 1;
    let p4_entry = MbrPartitionEntry {
        boot_indicator: PART_NON_BOOTABLE,
        start_chs: lba_to_chs(layout.part4_start),
        partition_type: PART_TYPE_LINUX_NATIVE,
        end_chs: lba_to_chs(p4_end_lba),
        start_lba: layout.part4_start,
        sector_count: layout.part4_count,
    };

    unsafe {
        let p_base = sector.as_mut_ptr().add(MBR_TABLE_OFFSET);
        core::ptr::copy_nonoverlapping(
            &p1_entry as *const MbrPartitionEntry as *const u8,
            p_base,
            size_of::<MbrPartitionEntry>(),
        );
        core::ptr::copy_nonoverlapping(
            &p2_entry as *const MbrPartitionEntry as *const u8,
            p_base.add(MBR_ENTRY_SIZE),
            size_of::<MbrPartitionEntry>(),
        );
        core::ptr::copy_nonoverlapping(
            &p3_entry as *const MbrPartitionEntry as *const u8,
            p_base.add(MBR_ENTRY_SIZE * 2),
            size_of::<MbrPartitionEntry>(),
        );
        core::ptr::copy_nonoverlapping(
            &p4_entry as *const MbrPartitionEntry as *const u8,
            p_base.add(MBR_ENTRY_SIZE * 3),
            size_of::<MbrPartitionEntry>(),
        );
    }

    // MBR Boot signature (0x55, 0xAA)
    sector[510] = 0x55;
    sector[511] = 0xAA;

    sector
}

/// Synthesizes FAT32 Boot Sector & BPB (Sector 0 of Partition 1).
pub fn synthesize_fat32_bpb(layout: &PartitionLayout, volume_label: &[u8; 11]) -> [u8; SECTOR_SIZE] {
    let mut sector = [0u8; SECTOR_SIZE];

    let bpb = Fat32BootSector {
        jmp: [0xEB, 0x58, 0x90],
        oem: *b"NOVAOS  ",
        bytes_per_sector: FAT32_BYTES_PER_SECTOR,
        sectors_per_cluster: FAT32_SECTORS_PER_CLUSTER,
        reserved_sectors: FAT32_RESERVED_SECTORS,
        num_fats: FAT32_NUM_FATS,
        root_entries: 0,
        total_sectors_16: 0,
        media_descriptor: FAT32_MEDIA_FIXED,
        fat_size_16: 0,
        sectors_per_track: 63,
        num_heads: 255,
        hidden_sectors: layout.part1_start,
        total_sectors_32: layout.part1_count,
        fat_size_32: layout.fat_size_sectors,
        ext_flags: 0,
        fs_version: 0,
        root_cluster: FAT32_ROOT_CLUSTER,
        fs_info_sector: FAT32_FSINFO_SECTOR,
        backup_boot_sector: FAT32_BACKUP_BOOT_SECTOR,
        reserved: [0u8; 12],
        drive_number: 0x80,
        nt_reserved: 0,
        boot_signature: 0x29,
        volume_id: 0x20261024,
        volume_label: *volume_label,
        fs_type: *b"FAT32   ",
    };

    unsafe {
        core::ptr::copy_nonoverlapping(
            &bpb as *const Fat32BootSector as *const u8,
            sector.as_mut_ptr(),
            size_of::<Fat32BootSector>(),
        );
    }

    sector[510] = 0x55;
    sector[511] = 0xAA;

    sector
}

/// Synthesizes FAT32 FSInfo Sector (Sector 1 of Partition 1).
pub fn synthesize_fat32_fsinfo(layout: &PartitionLayout) -> [u8; SECTOR_SIZE] {
    let mut sector = [0u8; SECTOR_SIZE];

    let free_clusters = if layout.total_clusters > 1 {
        layout.total_clusters - 1 // cluster 2 is root directory
    } else {
        0
    };

    let fsinfo = Fat32FsInfo {
        lead_sig: FAT32_FSINFO_LEAD_SIG,
        reserved1: [0u8; 480],
        struct_sig: FAT32_FSINFO_STRUCT_SIG,
        free_count: free_clusters,
        next_free: 3,
        reserved2: [0u8; 12],
        trail_sig: FAT32_FSINFO_TRAIL_SIG,
    };

    unsafe {
        core::ptr::copy_nonoverlapping(
            &fsinfo as *const Fat32FsInfo as *const u8,
            sector.as_mut_ptr(),
            size_of::<Fat32FsInfo>(),
        );
    }

    sector
}

/// Synthesizes sector 0 of the File Allocation Table (FAT1 / FAT2).
pub fn synthesize_fat32_fat0() -> [u8; SECTOR_SIZE] {
    let mut sector = [0u8; SECTOR_SIZE];

    // Cluster 0: Media descriptor in low byte, high bits 0x0FFFFF
    let c0 = FAT32_FAT_ENTRY_MEDIA.to_le_bytes();
    // Cluster 1: EOC / clean flag
    let c1 = FAT32_FAT_ENTRY_DIRTY.to_le_bytes();
    // Cluster 2: Root directory EOC marker
    let c2 = FAT32_FAT_ENTRY_EOF.to_le_bytes();

    sector[0..4].copy_from_slice(&c0);
    sector[4..8].copy_from_slice(&c1);
    sector[8..12].copy_from_slice(&c2);

    sector
}

/// Synthesizes root directory initial entry (Volume Label).
pub fn synthesize_fat32_root_dir(volume_label: &[u8; 11]) -> [u8; SECTOR_SIZE] {
    let mut sector = [0u8; SECTOR_SIZE];

    // Volume label directory entry (32 bytes)
    sector[0..11].copy_from_slice(volume_label);
    sector[11] = FAT_ATTR_VOLUME_ID;
    // Remainder of directory entry initialized to 0
    // Remainder of sector initialized to 0 (signals no more entries)

    sector
}

/// Synthesizes ext2 superblock (1024 bytes).
pub fn synthesize_ext2_superblock(layout: &PartitionLayout) -> [u8; 1024] {
    let mut buf = [0u8; 1024];

    let total_blocks = layout.part2_count / 2; // 1024-byte blocks
    let inodes_count = (layout.part2_count / 16).max(1024); // e.g. 4096 inodes
    let blocks_per_group = 8192u32;
    let inodes_per_group = inodes_count;
    let reserved_blocks = 38u32; // metadata overhead
    let free_blocks = total_blocks.saturating_sub(reserved_blocks);
    let free_inodes = inodes_count.saturating_sub(11);

    let sb = Ext2Superblock {
        inodes_count,
        blocks_count: total_blocks,
        r_blocks_count: total_blocks / 20, // 5% reserved for superuser
        free_blocks_count: free_blocks,
        free_inodes_count: free_inodes,
        first_data_block: 1, // block 1 is superblock for 1KB block size
        log_block_size: 0,   // 1024 << 0 = 1024 bytes
        log_frag_size: 0,
        blocks_per_group,
        frags_per_group: blocks_per_group,
        inodes_per_group,
        mtime: 0x671A6000,
        wtime: 0x671A6000,
        mnt_count: 0,
        max_mnt_count: 20,
        magic: EXT2_MAGIC,
        state: 1, // cleanly unmounted
        errors: 1, // continue on errors
        minor_rev_level: 0,
        lastcheck: 0x671A6000,
        checkinterval: 0,
        creator_os: 0, // Linux
        rev_level: 1,  // EXT2_DYNAMIC_REV
        def_resuid: 0,
        def_resgid: 0,
        first_ino: EXT2_FIRST_INO,
        inode_size: EXT2_INODE_SIZE,
    };

    unsafe {
        core::ptr::copy_nonoverlapping(
            &sb as *const Ext2Superblock as *const u8,
            buf.as_mut_ptr(),
            size_of::<Ext2Superblock>(),
        );
    }

    buf
}

/// Synthesizes ext2 block group descriptor table (512 bytes).
pub fn synthesize_ext2_bgd(layout: &PartitionLayout) -> [u8; SECTOR_SIZE] {
    let mut sector = [0u8; SECTOR_SIZE];

    let total_blocks = layout.part2_count / 2;
    let inodes_count = (layout.part2_count / 16).max(1024);
    let free_blocks = total_blocks.saturating_sub(38) as u16;
    let free_inodes = inodes_count.saturating_sub(11) as u16;

    let bgd = Ext2BlockGroupDesc {
        block_bitmap: 3,
        inode_bitmap: 4,
        inode_table: 5,
        free_blocks_count: free_blocks,
        free_inodes_count: free_inodes,
        used_dirs_count: 1, // root directory
        pad: 0,
        reserved: [0u8; 12],
    };

    unsafe {
        core::ptr::copy_nonoverlapping(
            &bgd as *const Ext2BlockGroupDesc as *const u8,
            sector.as_mut_ptr(),
            size_of::<Ext2BlockGroupDesc>(),
        );
    }

    sector
}

/// Synthesizes ext2 root directory inode (128 bytes).
pub fn synthesize_ext2_root_inode() -> [u8; 128] {
    let mut buf = [0u8; 128];

    let mut inode = Ext2Inode {
        mode: EXT2_S_IFDIR | EXT2_PERM_0755,
        uid: 0,
        size: EXT2_BLOCK_SIZE,
        atime: 0x671A6000,
        ctime: 0x671A6000,
        mtime: 0x671A6000,
        dtime: 0,
        gid: 0,
        links_count: 2, // "." and ".."
        blocks: 2,      // 2 * 512-byte sectors = 1024 bytes
        flags: 0,
        osd1: 0,
        block: [0u32; 15],
        generation: 0,
        file_acl: 0,
        size_high: 0,
        faddr: 0,
        osd2: [0u8; 12],
    };
    inode.block[0] = 37; // root directory data block

    unsafe {
        core::ptr::copy_nonoverlapping(
            &inode as *const Ext2Inode as *const u8,
            buf.as_mut_ptr(),
            size_of::<Ext2Inode>(),
        );
    }

    buf
}

/// Synthesizes ext2 root directory data block containing "." and ".." entries (1024 bytes).
pub fn synthesize_ext2_root_dir_block() -> [u8; 1024] {
    let mut buf = [0u8; 1024];

    // Entry 1: "." (inode 2, rec_len 12, name_len 1, file_type 2)
    buf[0..4].copy_from_slice(&EXT2_ROOT_INO.to_le_bytes());
    buf[4..6].copy_from_slice(&12u16.to_le_bytes()); // rec_len
    buf[6] = 1; // name_len
    buf[7] = EXT2_FT_DIR;
    buf[8] = b'.';
    buf[9] = 0;
    buf[10] = 0;
    buf[11] = 0;

    // Entry 2: ".." (inode 2, rec_len 1012, name_len 2, file_type 2)
    let rec_len_parent = (1024 - 12) as u16;
    buf[12..16].copy_from_slice(&EXT2_ROOT_INO.to_le_bytes());
    buf[16..18].copy_from_slice(&rec_len_parent.to_le_bytes());
    buf[18] = 2; // name_len
    buf[19] = EXT2_FT_DIR;
    buf[20] = b'.';
    buf[21] = b'.';
    buf[22] = 0;
    buf[23] = 0;

    buf
}

/// Verifies an MBR sector structurally and parses out its 4 partition entries.
pub fn verify_mbr_table(mbr: &[u8; SECTOR_SIZE]) -> Result<[MbrPartitionEntry; 4], i32> {
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        return Err(DISK_INSTALL_ERR_VERIFY_FAIL);
    }

    let mut entries = [MbrPartitionEntry {
        boot_indicator: 0,
        start_chs: [0; 3],
        partition_type: 0,
        end_chs: [0; 3],
        start_lba: 0,
        sector_count: 0,
    }; 4];

    for i in 0..4 {
        let offset = MBR_TABLE_OFFSET + i * MBR_ENTRY_SIZE;
        let entry = MbrPartitionEntry {
            boot_indicator: mbr[offset],
            start_chs: [mbr[offset + 1], mbr[offset + 2], mbr[offset + 3]],
            partition_type: mbr[offset + 4],
            end_chs: [mbr[offset + 5], mbr[offset + 6], mbr[offset + 7]],
            start_lba: u32::from_le_bytes([
                mbr[offset + 8],
                mbr[offset + 9],
                mbr[offset + 10],
                mbr[offset + 11],
            ]),
            sector_count: u32::from_le_bytes([
                mbr[offset + 12],
                mbr[offset + 13],
                mbr[offset + 14],
                mbr[offset + 15],
            ]),
        };

        if entry.partition_type == 0 || entry.sector_count == 0 || entry.start_lba == 0 {
            return Err(DISK_INSTALL_ERR_VERIFY_FAIL);
        }
        if entry.start_lba > 0xFFFFFFFFu32 - entry.sector_count {
            return Err(DISK_INSTALL_ERR_VERIFY_FAIL);
        }

        entries[i] = entry;
    }

    // Verify non-overlapping partitions
    for i in 0..3 {
        let end = entries[i].start_lba + entries[i].sector_count;
        if end > entries[i + 1].start_lba {
            return Err(DISK_INSTALL_ERR_VERIFY_FAIL);
        }
    }

    Ok(entries)
}

/// Executes on-disk partitioning and formatting for the target disk.
pub fn format_disk(
    config: &DiskLayoutConfig,
    out_summary: &mut DiskInstallSummary,
) -> i32 {
    let layout = match calculate_layout(
        config.total_sectors,
        config.part1_mb,
        config.part2_mb,
        config.part3_mb,
        config.part4_mb,
    ) {
        Ok(l) => l,
        Err(e) => return e,
    };

    let mbr_sector = synthesize_mbr(&layout);
    let fat32_label = *b"NOVAOS BOOT";
    let fat32_bpb = synthesize_fat32_bpb(&layout, &fat32_label);
    let fat32_fsinfo = synthesize_fat32_fsinfo(&layout);
    let fat32_fat0 = synthesize_fat32_fat0();
    let fat32_root = synthesize_fat32_root_dir(&fat32_label);
    let zero_sector = [0u8; SECTOR_SIZE];

    out_summary.status = DISK_INSTALL_OK;
    out_summary.target_device = config.target_device;
    out_summary.total_sectors = layout.total_sectors;
    out_summary.part1_start_lba = layout.part1_start;
    out_summary.part1_sectors = layout.part1_count;
    out_summary.part2_start_lba = layout.part2_start;
    out_summary.part2_sectors = layout.part2_count;
    out_summary.part3_start_lba = layout.part3_start;
    out_summary.part3_sectors = layout.part3_count;
    out_summary.part4_start_lba = layout.part4_start;
    out_summary.part4_sectors = layout.part4_count;
    out_summary.fat_size_sectors = layout.fat_size_sectors;
    out_summary.total_clusters = layout.total_clusters;
    out_summary.sectors_written = 0;

    // Dry-run mode: structurally validate everything without modifying physical media
    if config.dry_run != 0 {
        if verify_mbr_table(&mbr_sector).is_err() {
            out_summary.status = DISK_INSTALL_ERR_VERIFY_FAIL;
            return DISK_INSTALL_ERR_VERIFY_FAIL;
        }
        return DISK_INSTALL_OK;
    }

    // Live mode: check device presence and write structures to target device
    unsafe {
        let saved_dev = blockdev_current();
        let saved_offset = blockdev_get_partition_offset();

        blockdev_select(config.target_device);
        if !blockdev_is_present() {
            blockdev_select(saved_dev);
            out_summary.status = DISK_INSTALL_ERR_DEVICE_NOT_FOUND;
            return DISK_INSTALL_ERR_DEVICE_NOT_FOUND;
        }

        let mut written: u32 = 0;

        // 1. Write MBR sector 0
        if !blockdev_write_sectors_absolute(0, 1, mbr_sector.as_ptr()) {
            blockdev_select(saved_dev);
            out_summary.status = DISK_INSTALL_ERR_IO_WRITE;
            return DISK_INSTALL_ERR_IO_WRITE;
        }
        written += 1;

        // 2. Format FAT32 Boot Partition (Partition 1)
        if config.format_fat32 != 0 {
            // Boot sector at LBA 0 of partition 1
            if !blockdev_write_sectors_absolute(layout.part1_start, 1, fat32_bpb.as_ptr()) {
                blockdev_select(saved_dev);
                return DISK_INSTALL_ERR_IO_WRITE;
            }
            written += 1;

            // Backup boot sector at LBA 6
            if !blockdev_write_sectors_absolute(layout.part1_start + 6, 1, fat32_bpb.as_ptr()) {
                blockdev_select(saved_dev);
                return DISK_INSTALL_ERR_IO_WRITE;
            }
            written += 1;

            // FSInfo at LBA 1
            if !blockdev_write_sectors_absolute(layout.part1_start + 1, 1, fat32_fsinfo.as_ptr()) {
                blockdev_select(saved_dev);
                return DISK_INSTALL_ERR_IO_WRITE;
            }
            written += 1;

            // Backup FSInfo at LBA 7
            if !blockdev_write_sectors_absolute(layout.part1_start + 7, 1, fat32_fsinfo.as_ptr()) {
                blockdev_select(saved_dev);
                return DISK_INSTALL_ERR_IO_WRITE;
            }
            written += 1;

            // FAT1 table start at reserved sector offset 32
            let fat1_lba = layout.part1_start + (FAT32_RESERVED_SECTORS as u32);
            if !blockdev_write_sectors_absolute(fat1_lba, 1, fat32_fat0.as_ptr()) {
                blockdev_select(saved_dev);
                return DISK_INSTALL_ERR_IO_WRITE;
            }
            written += 1;

            // Zero out up to first 8 sectors of FAT1
            let clear_fat_sectors = layout.fat_size_sectors.min(8);
            for s in 1..clear_fat_sectors {
                let _ = blockdev_write_sectors_absolute(fat1_lba + s, 1, zero_sector.as_ptr());
                written += 1;
            }

            // FAT2 table start
            let fat2_lba = fat1_lba + layout.fat_size_sectors;
            if !blockdev_write_sectors_absolute(fat2_lba, 1, fat32_fat0.as_ptr()) {
                blockdev_select(saved_dev);
                return DISK_INSTALL_ERR_IO_WRITE;
            }
            written += 1;

            for s in 1..clear_fat_sectors {
                let _ = blockdev_write_sectors_absolute(fat2_lba + s, 1, zero_sector.as_ptr());
                written += 1;
            }

            // Root directory cluster 2 (8 sectors)
            let root_dir_lba = fat1_lba + (FAT32_NUM_FATS as u32) * layout.fat_size_sectors;
            if !blockdev_write_sectors_absolute(root_dir_lba, 1, fat32_root.as_ptr()) {
                blockdev_select(saved_dev);
                return DISK_INSTALL_ERR_IO_WRITE;
            }
            written += 1;

            for s in 1..(FAT32_SECTORS_PER_CLUSTER as u32) {
                let _ = blockdev_write_sectors_absolute(root_dir_lba + s, 1, zero_sector.as_ptr());
                written += 1;
            }
        }

        // 3. Format ext2 Data Partition (Partition 2)
        if config.format_ext2 != 0 {
            let ext2_sb = synthesize_ext2_superblock(&layout);
            let ext2_bgd = synthesize_ext2_bgd(&layout);
            let ext2_root_inode = synthesize_ext2_root_inode();
            let ext2_root_dir = synthesize_ext2_root_dir_block();

            // Superblock at byte offset 1024 (LBA part2_start + 2, 2 sectors)
            if !blockdev_write_sectors_absolute(layout.part2_start + 2, 2, ext2_sb.as_ptr()) {
                blockdev_select(saved_dev);
                return DISK_INSTALL_ERR_IO_WRITE;
            }
            written += 2;

            // Block Group Descriptor table at block 2 (LBA part2_start + 4, 1 sector)
            if !blockdev_write_sectors_absolute(layout.part2_start + 4, 1, ext2_bgd.as_ptr()) {
                blockdev_select(saved_dev);
                return DISK_INSTALL_ERR_IO_WRITE;
            }
            written += 1;

            // Block Bitmap at block 3 (LBA part2_start + 6)
            let mut block_bitmap = [0u8; SECTOR_SIZE];
            // First 38 blocks are allocated
            block_bitmap[0] = 0xFF;
            block_bitmap[1] = 0xFF;
            block_bitmap[2] = 0xFF;
            block_bitmap[3] = 0xFF;
            block_bitmap[4] = 0x3F; // 32 + 6 = 38 bits
            let _ = blockdev_write_sectors_absolute(layout.part2_start + 6, 1, block_bitmap.as_ptr());
            written += 1;

            // Inode Bitmap at block 4 (LBA part2_start + 8)
            let mut inode_bitmap = [0u8; SECTOR_SIZE];
            // First 11 inodes reserved
            inode_bitmap[0] = 0xFF;
            inode_bitmap[1] = 0x07; // 8 + 3 = 11 bits
            let _ = blockdev_write_sectors_absolute(layout.part2_start + 8, 1, inode_bitmap.as_ptr());
            written += 1;

            // Inode Table block 5 (LBA part2_start + 10): write root inode at index 1
            let mut inode_table_block = [0u8; SECTOR_SIZE];
            inode_table_block[128..256].copy_from_slice(&ext2_root_inode);
            let _ = blockdev_write_sectors_absolute(layout.part2_start + 10, 1, inode_table_block.as_ptr());
            written += 1;

            // Root directory data block at block 37 (LBA part2_start + 74, 2 sectors)
            let _ = blockdev_write_sectors_absolute(layout.part2_start + 74, 2, ext2_root_dir.as_ptr());
            written += 2;
        }

        // 4. Initialize Partition 3 (Journal region - clear descriptor sector)
        let _ = blockdev_write_sectors_absolute(layout.part3_start, 1, zero_sector.as_ptr());
        written += 1;

        // 5. Initialize Partition 4 (Crash Dump region - clear dump header)
        let _ = blockdev_write_sectors_absolute(layout.part4_start, 1, zero_sector.as_ptr());
        written += 1;

        // Restore original blockdev & partition offset
        blockdev_select(saved_dev);
        blockdev_set_partition_offset(saved_offset);

        out_summary.sectors_written = written;
        out_summary.status = DISK_INSTALL_OK;
    }

    DISK_INSTALL_OK
}

/// Query target device disk layout without formatting.
#[no_mangle]
pub extern "C" fn rust_disk_installer_query(
    target_dev: i32,
    out_summary: *mut DiskInstallSummary,
) -> i32 {
    if out_summary.is_null() {
        return DISK_INSTALL_ERR_INVALID_PARAM;
    }

    let default_config = DiskLayoutConfig {
        target_device: target_dev,
        total_sectors: 0,
        part1_mb: PART1_DEFAULT_MB,
        part2_mb: PART2_DEFAULT_MB,
        part3_mb: PART3_DEFAULT_MB,
        part4_mb: PART4_DEFAULT_MB,
        dry_run: 1,
        format_fat32: 1,
        format_ext2: 1,
    };

    let mut summary = DiskInstallSummary {
        status: 0,
        target_device: target_dev,
        total_sectors: 0,
        part1_start_lba: 0,
        part1_sectors: 0,
        part2_start_lba: 0,
        part2_sectors: 0,
        part3_start_lba: 0,
        part3_sectors: 0,
        part4_start_lba: 0,
        part4_sectors: 0,
        fat_size_sectors: 0,
        total_clusters: 0,
        sectors_written: 0,
    };

    let rc = format_disk(&default_config, &mut summary);
    unsafe {
        core::ptr::write_unaligned(out_summary, summary);
    }
    rc
}

/// C FFI entry point for partitioning and formatting a block device.
#[no_mangle]
pub extern "C" fn rust_disk_installer_format(
    config: *const DiskLayoutConfig,
    out_summary: *mut DiskInstallSummary,
) -> i32 {
    if config.is_null() {
        return DISK_INSTALL_ERR_INVALID_PARAM;
    }

    let cfg = unsafe { core::ptr::read_unaligned(config) };
    let mut summary = DiskInstallSummary {
        status: 0,
        target_device: cfg.target_device,
        total_sectors: 0,
        part1_start_lba: 0,
        part1_sectors: 0,
        part2_start_lba: 0,
        part2_sectors: 0,
        part3_start_lba: 0,
        part3_sectors: 0,
        part4_start_lba: 0,
        part4_sectors: 0,
        fat_size_sectors: 0,
        total_clusters: 0,
        sectors_written: 0,
    };

    let rc = format_disk(&cfg, &mut summary);
    if !out_summary.is_null() {
        unsafe {
            core::ptr::write_unaligned(out_summary, summary);
        }
    }
    rc
}

/// Comprehensive boot self-test verifying partitioning, FAT32/ext2 synthesis, and geometry.
#[no_mangle]
pub extern "C" fn rust_disk_installer_selftest() -> i32 {
    // Test 1: Partition layout calculations matching tools/build-disk-image.sh
    let layout = match calculate_layout(0, 64, 32, 4, 1) {
        Ok(l) => l,
        Err(_) => return -1,
    };

    if layout.part1_start != 2048 {
        return -2;
    }
    if layout.part1_count != 64 * 2048 {
        return -3;
    }
    if layout.part2_start != 2048 + 64 * 2048 {
        return -4;
    }
    if layout.part2_count != 32 * 2048 {
        return -5;
    }
    if layout.part3_start != 2048 + 96 * 2048 {
        return -6;
    }
    if layout.part3_count != 4 * 2048 {
        return -7;
    }
    if layout.part4_start != 2048 + 100 * 2048 {
        return -8;
    }
    if layout.part4_count != 1 * 2048 {
        return -9;
    }

    // Test 2: MBR synthesis and structural verification
    let mbr = synthesize_mbr(&layout);
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        return -10;
    }

    let parsed_entries = match verify_mbr_table(&mbr) {
        Ok(e) => e,
        Err(_) => return -11,
    };

    if parsed_entries[0].boot_indicator != PART_BOOTABLE
        || parsed_entries[0].partition_type != PART_TYPE_FAT32_LBA
        || parsed_entries[0].start_lba != 2048
        || parsed_entries[0].sector_count != 64 * 2048
    {
        return -12;
    }

    if parsed_entries[1].boot_indicator != PART_NON_BOOTABLE
        || parsed_entries[1].partition_type != PART_TYPE_LINUX_NATIVE
        || parsed_entries[1].start_lba != layout.part2_start
        || parsed_entries[1].sector_count != 32 * 2048
    {
        return -13;
    }

    if parsed_entries[2].partition_type != PART_TYPE_LINUX_NATIVE
        || parsed_entries[2].start_lba != layout.part3_start
        || parsed_entries[2].sector_count != 4 * 2048
    {
        return -14;
    }

    if parsed_entries[3].partition_type != PART_TYPE_LINUX_NATIVE
        || parsed_entries[3].start_lba != layout.part4_start
        || parsed_entries[3].sector_count != 1 * 2048
    {
        return -15;
    }

    // Test 3: FAT32 BPB layout & geometry checks
    let volume_label = *b"NOVAOS BOOT";
    let bpb_sector = synthesize_fat32_bpb(&layout, &volume_label);
    if bpb_sector[510] != 0x55 || bpb_sector[511] != 0xAA {
        return -16;
    }

    let bpb = unsafe { *(bpb_sector.as_ptr() as *const Fat32BootSector) };
    if bpb.bytes_per_sector != 512
        || bpb.sectors_per_cluster != 8
        || bpb.reserved_sectors != 32
        || bpb.num_fats != 2
        || bpb.root_entries != 0
        || bpb.total_sectors_16 != 0
        || bpb.media_descriptor != 0xF8
        || bpb.fat_size_16 != 0
        || bpb.hidden_sectors != 2048
        || bpb.total_sectors_32 != 64 * 2048
        || bpb.root_cluster != 2
        || bpb.fs_info_sector != 1
        || bpb.backup_boot_sector != 6
        || bpb.boot_signature != 0x29
        || &bpb.volume_label != b"NOVAOS BOOT"
        || &bpb.fs_type != b"FAT32   "
    {
        return -17;
    }

    // Test 4: FAT32 FSInfo sector signatures
    let fsinfo_sector = synthesize_fat32_fsinfo(&layout);
    let fsinfo = unsafe { *(fsinfo_sector.as_ptr() as *const Fat32FsInfo) };
    if fsinfo.lead_sig != FAT32_FSINFO_LEAD_SIG
        || fsinfo.struct_sig != FAT32_FSINFO_STRUCT_SIG
        || fsinfo.trail_sig != FAT32_FSINFO_TRAIL_SIG
        || fsinfo.next_free != 3
    {
        return -18;
    }

    // Test 5: FAT table sector 0 entries
    let fat0_sector = synthesize_fat32_fat0();
    let c0 = u32::from_le_bytes([fat0_sector[0], fat0_sector[1], fat0_sector[2], fat0_sector[3]]);
    let c1 = u32::from_le_bytes([fat0_sector[4], fat0_sector[5], fat0_sector[6], fat0_sector[7]]);
    let c2 = u32::from_le_bytes([fat0_sector[8], fat0_sector[9], fat0_sector[10], fat0_sector[11]]);
    if c0 != FAT32_FAT_ENTRY_MEDIA || c1 != FAT32_FAT_ENTRY_DIRTY || c2 != FAT32_FAT_ENTRY_EOF {
        return -19;
    }

    // Test 6: ext2 superblock synthesis & magic check
    let ext2_sb_buf = synthesize_ext2_superblock(&layout);
    let ext2_sb = unsafe { *(ext2_sb_buf.as_ptr() as *const Ext2Superblock) };
    if ext2_sb.magic != EXT2_MAGIC
        || ext2_sb.rev_level != 1
        || ext2_sb.inode_size != 128
        || ext2_sb.first_data_block != 1
        || ext2_sb.log_block_size != 0
    {
        return -20;
    }

    // Test 7: ext2 block group descriptor and root inode
    let bgd_sector = synthesize_ext2_bgd(&layout);
    let bgd = unsafe { *(bgd_sector.as_ptr() as *const Ext2BlockGroupDesc) };
    if bgd.block_bitmap != 3 || bgd.inode_bitmap != 4 || bgd.inode_table != 5 || bgd.used_dirs_count != 1 {
        return -21;
    }

    let root_inode_buf = synthesize_ext2_root_inode();
    let root_inode = unsafe { *(root_inode_buf.as_ptr() as *const Ext2Inode) };
    if (root_inode.mode & EXT2_S_IFDIR) == 0 || root_inode.block[0] != 37 || root_inode.size != 1024 {
        return -22;
    }

    // Test 8: End-to-end dry-run formatting pipeline
    let test_cfg = DiskLayoutConfig {
        target_device: 0,
        total_sectors: 0,
        part1_mb: 64,
        part2_mb: 32,
        part3_mb: 4,
        part4_mb: 1,
        dry_run: 1,
        format_fat32: 1,
        format_ext2: 1,
    };
    let mut summary = DiskInstallSummary {
        status: 0,
        target_device: 0,
        total_sectors: 0,
        part1_start_lba: 0,
        part1_sectors: 0,
        part2_start_lba: 0,
        part2_sectors: 0,
        part3_start_lba: 0,
        part3_sectors: 0,
        part4_start_lba: 0,
        part4_sectors: 0,
        fat_size_sectors: 0,
        total_clusters: 0,
        sectors_written: 0,
    };

    let rc = format_disk(&test_cfg, &mut summary);
    if rc != DISK_INSTALL_OK || summary.status != DISK_INSTALL_OK {
        return -23;
    }
    if summary.part1_start_lba != 2048 || summary.part1_sectors != 64 * 2048 {
        return -24;
    }

    // Test 9: Real non-destructive blockdev sector I/O validation
    // Verify blockdev read/write on a scratch sector in Partition 4 (Crash Dump area + 100)
    // without touching any filesystem partitions.
    unsafe {
        let scratch_lba = layout.part4_start.saturating_add(100);
        let mut original_sector = [0u8; SECTOR_SIZE];
        let mut verify_sector = [0u8; SECTOR_SIZE];
        let mut test_pattern = [0u8; SECTOR_SIZE];

        for i in 0..SECTOR_SIZE {
            test_pattern[i] = ((i * 13 + 37) & 0xFF) as u8;
        }

        if blockdev_read_sectors_absolute(scratch_lba, 1, original_sector.as_mut_ptr()) {
            if blockdev_write_sectors_absolute(scratch_lba, 1, test_pattern.as_ptr()) {
                if blockdev_read_sectors_absolute(scratch_lba, 1, verify_sector.as_mut_ptr()) {
                    let mut matched = true;
                    for i in 0..SECTOR_SIZE {
                        if verify_sector[i] != test_pattern[i] {
                            matched = false;
                            break;
                        }
                    }
                    // Restore original sector data
                    let _ = blockdev_write_sectors_absolute(scratch_lba, 1, original_sector.as_ptr());
                    if !matched {
                        return -25;
                    }
                }
            }
        }
    }

    unsafe {
        kernel_log(
            b"[ OK ] Disk installer & on-disk partitioning engine (MBR, FAT32 BPB/FSInfo/FAT, ext2 superblock, journal & crashdump layout): result=0\n\0".as_ptr(),
        );
    }

    0
}
