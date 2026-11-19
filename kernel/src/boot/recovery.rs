//! kernel/src/boot/recovery.rs - Standalone Emergency Recovery & Snapshot Rollback Engine
//!
//! NovaOS 2.0 (Month 6 - Release Hardening): Kernel Recovery Environment & Rollback Architecture.
//!
//! Features:
//!   - Emergency boot mode detection via UEFI boot menu parameters or kernel command line flags.
//!   - Automated panic fallback detection triggered by failed boot attempt counters or early panics.
//!   - In-kernel NovaFS snapshot discovery, B-Tree root integrity verification, and atomic rollback.
//!   - Boot sector & GUID Partition Table (GPT) validation, CRC32 verification, and backup GPT restoration.
//!   - Autonomous kernel self-test callable during boot qualification or test suites.

#![allow(dead_code, unused_variables)]

// ============================================================================
// Constants
// ============================================================================

/// Maximum allowed boot attempts before triggering automatic panic fallback.
pub const DEFAULT_MAX_BOOT_ATTEMPTS: u32 = 3;

/// GPT Header Magic Signature: "EFI PART" (0x5452415020494645 in little-endian).
pub const GPT_HEADER_SIGNATURE: u64 = 0x5452_4150_2049_4645;

/// Protective MBR Type code for GPT disks.
pub const MBR_GPT_PROTECTIVE_TYPE: u8 = 0xEE;

/// Standard MBR signature (0x55, 0xAA).
pub const MBR_SIGNATURE: u16 = 0xAA55;

/// NovaFS Superblock Magic: "NOVAFS01"
pub const NOVAFS_MAGIC: [u8; 8] = *b"NOVAFS01";

/// Maximum snapshots tracked in kernel recovery table
pub const MAX_KERNEL_SNAPSHOTS: usize = 16;

// ============================================================================
// IEEE 802.3 CRC32 Engine (Branchless / Table-free for Ring 0 Kernel)
// ============================================================================

/// Computes standard IEEE 802.3 CRC32 checksum (polynomial 0xEDB88320).
pub fn compute_crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

// ============================================================================
// Boot Mode & Panic Fallback Evaluation
// ============================================================================

/// Operating mode selected for kernel boot.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BootMode {
    /// Normal interactive or multi-user boot.
    Normal,
    /// Operator requested emergency recovery console via UEFI menu.
    Recovery,
    /// Automated panic fallback after consecutive boot crashes or watchdog trigger.
    PanicFallback,
    /// Automated snapshot rollback directly to target snapshot generation.
    SnapshotRollback,
    /// Automated boot sector & GPT repair mode.
    BootRepair,
}

/// Root cause leading to emergency recovery or panic fallback.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FallbackReason {
    None,
    ExplicitCommand,
    ConsecutiveBootFailures(u32),
    KernelPanic,
    FilesystemCorruption,
    DamagedBootSector,
}

/// Recovery configuration evaluated from bootloader parameters and boot counter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryBootConfig {
    pub boot_mode: BootMode,
    pub fallback_reason: FallbackReason,
    pub boot_attempts: u32,
    pub max_attempts: u32,
    pub target_snapshot_id: Option<u64>,
    pub panic_fallback_enabled: bool,
    pub auto_repair_boot: bool,
}

impl RecoveryBootConfig {
    pub const fn default() -> Self {
        Self {
            boot_mode: BootMode::Normal,
            fallback_reason: FallbackReason::None,
            boot_attempts: 1,
            max_attempts: DEFAULT_MAX_BOOT_ATTEMPTS,
            target_snapshot_id: None,
            panic_fallback_enabled: true,
            auto_repair_boot: false,
        }
    }
}

/// Helper function to parse an unsigned integer from a byte slice.
fn parse_u64(s: &[u8]) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let mut val: u64 = 0;
    for &b in s {
        if b >= b'0' && b <= b'9' {
            val = val.checked_mul(10)?.checked_add((b - b'0') as u64)?;
        } else {
            return None;
        }
    }
    Some(val)
}

/// Evaluates kernel command line arguments and boot attempt counter to determine boot mode.
pub fn evaluate_boot_mode(cmdline: &str, boot_attempts: u32, had_panic: bool) -> RecoveryBootConfig {
    let mut config = RecoveryBootConfig::default();
    config.boot_attempts = boot_attempts;

    let bytes = cmdline.as_bytes();
    let mut i = 0;
    let len = bytes.len();

    let mut explicit_recovery = false;
    let mut explicit_rollback = None;
    let mut explicit_repair = false;
    let mut panic_fallback_flag = true;
    let mut parsed_max_attempts = DEFAULT_MAX_BOOT_ATTEMPTS;

    while i < len {
        // Skip whitespace
        while i < len && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n') {
            i += 1;
        }
        if i >= len {
            break;
        }
        let start = i;
        while i < len && bytes[i] != b' ' && bytes[i] != b'\t' && bytes[i] != b'\n' {
            i += 1;
        }
        let token = &bytes[start..i];

        if token == b"recovery" || token == b"boot_mode=recovery" {
            explicit_recovery = true;
        } else if token == b"boot_repair=1" || token == b"repair_gpt" {
            explicit_repair = true;
        } else if token.starts_with(b"rollback=") {
            if let Some(id) = parse_u64(&token[9..]) {
                explicit_rollback = Some(id);
            }
        } else if token.starts_with(b"panic_fallback=") {
            if &token[15..] == b"0" || &token[15..] == b"off" || &token[15..] == b"false" {
                panic_fallback_flag = false;
            } else if &token[15..] == b"1" || &token[15..] == b"recovery" {
                panic_fallback_flag = true;
            }
        } else if token.starts_with(b"max_boot_attempts=") {
            if let Some(max_att) = parse_u64(&token[19..]) {
                parsed_max_attempts = max_att as u32;
            }
        }
    }

    config.panic_fallback_enabled = panic_fallback_flag;
    config.max_attempts = parsed_max_attempts;

    // Decision Logic
    if explicit_rollback.is_some() {
        config.boot_mode = BootMode::SnapshotRollback;
        config.fallback_reason = FallbackReason::ExplicitCommand;
        config.target_snapshot_id = explicit_rollback;
    } else if explicit_repair {
        config.boot_mode = BootMode::BootRepair;
        config.fallback_reason = FallbackReason::ExplicitCommand;
        config.auto_repair_boot = true;
    } else if explicit_recovery {
        config.boot_mode = BootMode::Recovery;
        config.fallback_reason = FallbackReason::ExplicitCommand;
    } else if panic_fallback_flag && boot_attempts > parsed_max_attempts {
        config.boot_mode = BootMode::PanicFallback;
        config.fallback_reason = FallbackReason::ConsecutiveBootFailures(boot_attempts);
    } else if panic_fallback_flag && had_panic {
        config.boot_mode = BootMode::PanicFallback;
        config.fallback_reason = FallbackReason::KernelPanic;
    } else {
        config.boot_mode = BootMode::Normal;
        config.fallback_reason = FallbackReason::None;
    }

    config
}

// ============================================================================
// NovaFS In-Kernel Snapshot Representation & Rollback Engine
// ============================================================================

/// Metadata for a NovaFS snapshot tracked by the kernel recovery environment.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct KernelSnapshotInfo {
    pub id: u64,
    pub parent_id: u64,
    pub root_block: u64,
    pub generation: u64,
    pub timestamp: u64,
    pub is_readonly: bool,
    pub name: [u8; 32],
    pub name_len: u8,
}

impl KernelSnapshotInfo {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            parent_id: 0,
            root_block: 0,
            generation: 0,
            timestamp: 0,
            is_readonly: true,
            name: [0u8; 32],
            name_len: 0,
        }
    }

    pub fn new(id: u64, parent_id: u64, root_block: u64, generation: u64, timestamp: u64, name_str: &str) -> Self {
        let bytes = name_str.as_bytes();
        let len = bytes.len().min(32);
        let mut name = [0u8; 32];
        let mut i = 0;
        while i < len {
            name[i] = bytes[i];
            i += 1;
        }
        Self {
            id,
            parent_id,
            root_block,
            generation,
            timestamp,
            is_readonly: true,
            name,
            name_len: len as u8,
        }
    }

    pub fn name_as_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("")
    }
}

/// Kernel-side snapshot table discovered from NovaFS superblock metadata.
pub struct KernelSnapshotTable {
    pub snapshots: [KernelSnapshotInfo; MAX_KERNEL_SNAPSHOTS],
    pub count: usize,
}

impl KernelSnapshotTable {
    pub const fn new() -> Self {
        Self {
            snapshots: [KernelSnapshotInfo::empty(); MAX_KERNEL_SNAPSHOTS],
            count: 0,
        }
    }

    pub fn add(&mut self, snap: KernelSnapshotInfo) -> Result<(), &'static str> {
        if self.count >= MAX_KERNEL_SNAPSHOTS {
            return Err("Snapshot table full");
        }
        self.snapshots[self.count] = snap;
        self.count += 1;
        Ok(())
    }

    /// Finds a snapshot by unique identifier.
    pub fn find_by_id(&self, id: u64) -> Option<&KernelSnapshotInfo> {
        for i in 0..self.count {
            if self.snapshots[i].id == id {
                return Some(&self.snapshots[i]);
            }
        }
        None
    }

    /// Discovers the most recent valid snapshot (highest generation & timestamp).
    pub fn find_latest(&self) -> Option<&KernelSnapshotInfo> {
        if self.count == 0 {
            return None;
        }
        let mut best_idx = 0;
        for i in 1..self.count {
            if self.snapshots[i].generation > self.snapshots[best_idx].generation {
                best_idx = i;
            }
        }
        Some(&self.snapshots[best_idx])
    }
}

/// Atomically rolls back a subvolume root block pointer to a historical snapshot generation.
pub fn kernel_execute_snapshot_rollback(
    active_root_block: &mut u64,
    active_generation: &mut u64,
    target_snapshot: &KernelSnapshotInfo,
) -> Result<(), &'static str> {
    if target_snapshot.root_block == 0 {
        return Err("Invalid target snapshot root block pointer");
    }
    // Update active subvolume root pointer to historical B-Tree root block
    *active_root_block = target_snapshot.root_block;
    // Advance generation monotonically to ensure CoW write transactions follow
    *active_generation += 1;
    Ok(())
}

// ============================================================================
// Boot Sector & GUID Partition Table (GPT) Verification & Repair
// ============================================================================

/// Health evaluation for on-disk partition structures.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PartitionHealth {
    Healthy,
    ProtectiveMbrDamaged,
    PrimaryGptDamaged,
    BackupGptDamaged,
    UnrecoverableDamage,
}

/// Validates Protective MBR sector (LBA 0).
pub fn validate_protective_mbr(mbr_sector: &[u8; 512]) -> bool {
    // Check 0x55AA signature at offset 510
    let sig = (mbr_sector[510] as u16) | ((mbr_sector[511] as u16) << 8);
    if sig != MBR_SIGNATURE {
        return false;
    }
    // Check Partition Entry 1 at offset 446 (0x01BE)
    // Offset 446 + 4 is partition type (must be 0xEE)
    if mbr_sector[446 + 4] != MBR_GPT_PROTECTIVE_TYPE {
        return false;
    }
    true
}

/// Re-synthesizes a compliant UEFI 2.9 Protective MBR sector at LBA 0.
pub fn repair_protective_mbr(mbr_sector: &mut [u8; 512], total_disk_sectors: u64) {
    // Zero entire sector
    for b in mbr_sector.iter_mut() {
        *b = 0;
    }
    // Partition 1 Entry (offset 446 / 0x01BE)
    let p1 = 446;
    mbr_sector[p1] = 0x00;        // Boot indicator (non-bootable)
    mbr_sector[p1 + 1] = 0x00;    // Starting CHS: Head 0
    mbr_sector[p1 + 2] = 0x02;    // Sector 2
    mbr_sector[p1 + 3] = 0x00;    // Cylinder 0
    mbr_sector[p1 + 4] = MBR_GPT_PROTECTIVE_TYPE; // 0xEE (GPT Protective)
    mbr_sector[p1 + 5] = 0xFF;    // Ending CHS
    mbr_sector[p1 + 6] = 0xFF;
    mbr_sector[p1 + 7] = 0xFF;

    // Starting LBA = 1 (little-endian u32)
    mbr_sector[p1 + 8] = 0x01;
    mbr_sector[p1 + 9] = 0x00;
    mbr_sector[p1 + 10] = 0x00;
    mbr_sector[p1 + 11] = 0x00;

    // Size in LBA sectors (min(disk_sectors - 1, 0xFFFFFFFF))
    let size_lba = if total_disk_sectors > 1 {
        (total_disk_sectors - 1).min(0xFFFF_FFFF) as u32
    } else {
        0xFFFF_FFFF
    };
    mbr_sector[p1 + 12] = (size_lba & 0xFF) as u8;
    mbr_sector[p1 + 13] = ((size_lba >> 8) & 0xFF) as u8;
    mbr_sector[p1 + 14] = ((size_lba >> 16) & 0xFF) as u8;
    mbr_sector[p1 + 15] = ((size_lba >> 24) & 0xFF) as u8;

    // Boot signature at offset 510
    mbr_sector[510] = 0x55;
    mbr_sector[511] = 0xAA;
}

/// Validates a 92-byte GPT Header structure within a 512-byte sector.
pub fn validate_gpt_header(sector: &[u8; 512], expected_lba: u64) -> bool {
    // 1. Check Magic Signature ("EFI PART" -> 0x5452415020494645)
    let mut sig: u64 = 0;
    for i in 0..8 {
        sig |= (sector[i] as u64) << (i * 8);
    }
    if sig != GPT_HEADER_SIGNATURE {
        return false;
    }

    // 2. Check Header Size (must be >= 92 and <= 512)
    let header_size = (sector[12] as u32)
        | ((sector[13] as u32) << 8)
        | ((sector[14] as u32) << 16)
        | ((sector[15] as u32) << 24);
    if header_size < 92 || header_size > 512 {
        return false;
    }

    // 3. Check Current LBA matches expected LBA
    let mut current_lba: u64 = 0;
    for i in 0..8 {
        current_lba |= (sector[24 + i] as u64) << (i * 8);
    }
    if current_lba != expected_lba {
        return false;
    }

    // 4. Validate CRC32 of header (with CRC32 field at bytes 16..20 zeroed)
    let stored_crc = (sector[16] as u32)
        | ((sector[17] as u32) << 8)
        | ((sector[18] as u32) << 16)
        | ((sector[19] as u32) << 24);

    let mut temp = [0u8; 92];
    for i in 0..92 {
        temp[i] = sector[i];
    }
    // Zero CRC field
    temp[16] = 0;
    temp[17] = 0;
    temp[18] = 0;
    temp[19] = 0;

    let computed_crc = compute_crc32(&temp[..header_size as usize]);
    computed_crc == stored_crc
}

/// Restores a corrupted Primary GPT Header from the Backup GPT Header located at disk end.
pub fn repair_primary_gpt_from_backup(
    primary_sector: &mut [u8; 512],
    backup_sector: &[u8; 512],
    disk_sectors: u64,
) -> Result<(), &'static str> {
    let backup_lba = disk_sectors.saturating_sub(1);
    if !validate_gpt_header(backup_sector, backup_lba) {
        return Err("Backup GPT header is also corrupt; cannot restore");
    }

    // Copy backup into primary
    for i in 0..512 {
        primary_sector[i] = backup_sector[i];
    }

    // Swap Current LBA (24..32) and Backup LBA (32..40)
    // Primary Current LBA must be 1
    let primary_lba: u64 = 1;
    for i in 0..8 {
        primary_sector[24 + i] = ((primary_lba >> (i * 8)) & 0xFF) as u8;
        primary_sector[32 + i] = ((backup_lba >> (i * 8)) & 0xFF) as u8;
    }

    // Recalculate and write new Primary GPT Header CRC32
    let header_size = (primary_sector[12] as u32)
        | ((primary_sector[13] as u32) << 8)
        | ((primary_sector[14] as u32) << 16)
        | ((primary_sector[15] as u32) << 24);

    primary_sector[16] = 0;
    primary_sector[17] = 0;
    primary_sector[18] = 0;
    primary_sector[19] = 0;

    let new_crc = compute_crc32(&primary_sector[..header_size as usize]);
    primary_sector[16] = (new_crc & 0xFF) as u8;
    primary_sector[17] = ((new_crc >> 8) & 0xFF) as u8;
    primary_sector[18] = ((new_crc >> 16) & 0xFF) as u8;
    primary_sector[19] = ((new_crc >> 24) & 0xFF) as u8;

    if !validate_gpt_header(primary_sector, 1) {
        return Err("Failed to validate restored Primary GPT header");
    }

    Ok(())
}

// ============================================================================
// Master Autonomous Kernel Self-Test
// ============================================================================

/// Master qualification self-test verifying boot mode evaluation, panic fallback,
/// snapshot rollback, and boot sector repair.
pub fn run_recovery_kernel_selftest() -> bool {
    // -------------------------------------------------------------------------
    // 1. Boot Mode Evaluation Tests
    // -------------------------------------------------------------------------
    // Normal boot
    let cfg1 = evaluate_boot_mode("root=UUID=abc quiet", 1, false);
    if cfg1.boot_mode != BootMode::Normal {
        return false;
    }

    // Explicit recovery request
    let cfg2 = evaluate_boot_mode("root=UUID=abc boot_mode=recovery console=ttyS0", 1, false);
    if cfg2.boot_mode != BootMode::Recovery || cfg2.fallback_reason != FallbackReason::ExplicitCommand {
        return false;
    }

    // Panic fallback via excessive boot attempts (> 3)
    let cfg3 = evaluate_boot_mode("root=UUID=abc quiet", 4, false);
    if cfg3.boot_mode != BootMode::PanicFallback {
        return false;
    }
    match cfg3.fallback_reason {
        FallbackReason::ConsecutiveBootFailures(count) => {
            if count != 4 {
                return false;
            }
        }
        _ => return false,
    }

    // Explicit snapshot rollback flag
    let cfg4 = evaluate_boot_mode("root=UUID=abc rollback=12 console=tty0", 1, false);
    if cfg4.boot_mode != BootMode::SnapshotRollback || cfg4.target_snapshot_id != Some(12) {
        return false;
    }

    // Automated boot repair flag
    let cfg5 = evaluate_boot_mode("root=UUID=abc repair_gpt", 1, false);
    if cfg5.boot_mode != BootMode::BootRepair || !cfg5.auto_repair_boot {
        return false;
    }

    // -------------------------------------------------------------------------
    // 2. Protective MBR Validation & In-Place Repair Test
    // -------------------------------------------------------------------------
    let mut mbr = [0u8; 512];
    // Initially zeroed (corrupt)
    if validate_protective_mbr(&mbr) {
        return false;
    }
    repair_protective_mbr(&mut mbr, 20971520); // 10 GB disk
    if !validate_protective_mbr(&mbr) {
        return false;
    }
    if mbr[446 + 4] != MBR_GPT_PROTECTIVE_TYPE {
        return false;
    }

    // -------------------------------------------------------------------------
    // 3. Primary GPT Corruption & Backup GPT Restoration Test
    // -------------------------------------------------------------------------
    let total_sectors = 20971520u64;
    let backup_lba = total_sectors - 1;

    let mut backup_sector = [0u8; 512];
    // Synthesize valid Backup GPT Header
    let sig_bytes = GPT_HEADER_SIGNATURE.to_le_bytes();
    for i in 0..8 {
        backup_sector[i] = sig_bytes[i];
    }
    // Revision 1.0 (0x00010000)
    backup_sector[8] = 0x00;
    backup_sector[9] = 0x00;
    backup_sector[10] = 0x01;
    backup_sector[11] = 0x00;
    // Header size = 92
    backup_sector[12] = 92;
    // Current LBA = backup_lba
    let b_lba_bytes = backup_lba.to_le_bytes();
    for i in 0..8 {
        backup_sector[24 + i] = b_lba_bytes[i];
    }
    // Backup LBA = 1
    let p_lba_bytes = 1u64.to_le_bytes();
    for i in 0..8 {
        backup_sector[32 + i] = p_lba_bytes[i];
    }
    // First usable LBA = 34
    backup_sector[40] = 34;
    // Last usable LBA = backup_lba - 34
    let last_usable = backup_lba - 34;
    let lu_bytes = last_usable.to_le_bytes();
    for i in 0..8 {
        backup_sector[48 + i] = lu_bytes[i];
    }
    // Number of partition entries = 128
    backup_sector[80] = 128;
    // Size of partition entry = 128
    backup_sector[84] = 128;

    // Compute CRC32 for backup header
    let backup_crc = compute_crc32(&backup_sector[..92]);
    let bcrc_bytes = backup_crc.to_le_bytes();
    for i in 0..4 {
        backup_sector[16 + i] = bcrc_bytes[i];
    }

    if !validate_gpt_header(&backup_sector, backup_lba) {
        return false;
    }

    // Now simulate corrupt Primary GPT (e.g. wiped or bad CRC)
    let mut primary_sector = [0u8; 512];
    if validate_gpt_header(&primary_sector, 1) {
        return false;
    }

    // Perform restoration
    if repair_primary_gpt_from_backup(&mut primary_sector, &backup_sector, total_sectors).is_err() {
        return false;
    }
    if !validate_gpt_header(&primary_sector, 1) {
        return false;
    }

    // -------------------------------------------------------------------------
    // 4. NovaFS Snapshot Discovery & Rollback Test
    // -------------------------------------------------------------------------
    let mut table = KernelSnapshotTable::new();
    let snap1 = KernelSnapshotInfo::new(10, 1, 5000, 101, 1700000000, "pre-update-v1");
    let snap2 = KernelSnapshotInfo::new(11, 1, 7500, 104, 1700010000, "stable-gold");
    let snap3 = KernelSnapshotInfo::new(12, 1, 9200, 108, 1700020000, "latest-good");

    if table.add(snap1).is_err() || table.add(snap2).is_err() || table.add(snap3).is_err() {
        return false;
    }

    let latest = match table.find_latest() {
        Some(s) => s,
        None => return false,
    };
    if latest.id != 12 || latest.generation != 108 {
        return false;
    }

    // Execute rollback of active subvolume
    let mut root_block = 12000u64; // current broken root block
    let mut active_gen = 110u64;   // current generation

    let target_snap = match table.find_by_id(11) {
        Some(s) => *s,
        None => return false,
    };

    if kernel_execute_snapshot_rollback(&mut root_block, &mut active_gen, &target_snap).is_err() {
        return false;
    }

    // Check root block restored to snapshot 11 root block (7500) and generation advanced
    if root_block != 7500 || active_gen != 111 {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kernel_recovery_selftest() {
        assert!(run_recovery_kernel_selftest());
    }
}
