//! userland/desktop/apps/nova-recovery/src/boot_repair.rs
//!
//! Boot Sector, Protective MBR, and GUID Partition Table (GPT) Repair Subsystem.
//! Conforms to UEFI 2.9 specification for dual GPT header redundancy and CRC32 verification.

use crate::crypto::crc32;

pub const SECTOR_SIZE: usize = 512;
pub const GPT_HEADER_MAGIC: u64 = 0x5452_4150_2049_4645; // "EFI PART"
pub const MBR_GPT_TYPE: u8 = 0xEE;
pub const MBR_SIGNATURE: u16 = 0xAA55;

/// Partition Health Assessment Summary
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootHealthReport {
    pub mbr_valid: bool,
    pub primary_gpt_valid: bool,
    pub backup_gpt_valid: bool,
    pub partitions_found: usize,
    pub recommended_action: String,
}

/// Parsed GPT Header
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedGptHeader {
    pub signature: u64,
    pub revision: u32,
    pub header_size: u32,
    pub header_crc32: u32,
    pub current_lba: u64,
    pub backup_lba: u64,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
    pub disk_guid: [u8; 16],
    pub partition_entry_lba: u64,
    pub num_partition_entries: u32,
    pub size_partition_entry: u32,
    pub partition_entry_array_crc32: u32,
}

impl ParsedGptHeader {
    pub fn parse_sector(sector: &[u8]) -> Option<Self> {
        if sector.len() < 92 {
            return None;
        }

        let sig = u64::from_le_bytes(sector[0..8].try_into().ok()?);
        if sig != GPT_HEADER_MAGIC {
            return None;
        }

        let revision = u32::from_le_bytes(sector[8..12].try_into().ok()?);
        let header_size = u32::from_le_bytes(sector[12..16].try_into().ok()?);
        let header_crc32 = u32::from_le_bytes(sector[16..20].try_into().ok()?);
        let current_lba = u64::from_le_bytes(sector[24..32].try_into().ok()?);
        let backup_lba = u64::from_le_bytes(sector[32..40].try_into().ok()?);
        let first_usable_lba = u64::from_le_bytes(sector[40..48].try_into().ok()?);
        let last_usable_lba = u64::from_le_bytes(sector[48..56].try_into().ok()?);

        let mut disk_guid = [0u8; 16];
        disk_guid.copy_from_slice(&sector[56..72]);

        let partition_entry_lba = u64::from_le_bytes(sector[72..80].try_into().ok()?);
        let num_partition_entries = u32::from_le_bytes(sector[80..84].try_into().ok()?);
        let size_partition_entry = u32::from_le_bytes(sector[84..88].try_into().ok()?);
        let partition_entry_array_crc32 = u32::from_le_bytes(sector[88..92].try_into().ok()?);

        Some(Self {
            signature: sig,
            revision,
            header_size,
            header_crc32,
            current_lba,
            backup_lba,
            first_usable_lba,
            last_usable_lba,
            disk_guid,
            partition_entry_lba,
            num_partition_entries,
            size_partition_entry,
            partition_entry_array_crc32,
        })
    }

    pub fn is_crc_valid(&self, sector: &[u8]) -> bool {
        if sector.len() < self.header_size as usize {
            return false;
        }
        let mut zeroed = sector[..self.header_size as usize].to_vec();
        zeroed[16..20].copy_from_slice(&[0, 0, 0, 0]);
        let computed = crc32(&zeroed);
        computed == self.header_crc32
    }
}

/// Boot Repair Engine capable of analyzing and repairing disks
pub struct BootRepairManager {
    pub total_sectors: u64,
}

impl BootRepairManager {
    pub fn new(total_sectors: u64) -> Self {
        Self { total_sectors }
    }

    /// Validates Protective MBR sector at LBA 0
    pub fn validate_mbr(&self, lba0: &[u8]) -> bool {
        if lba0.len() < SECTOR_SIZE {
            return false;
        }
        let sig = u16::from_le_bytes([lba0[510], lba0[511]]);
        if sig != MBR_SIGNATURE {
            return false;
        }
        // Offset 446 + 4 is partition 1 type code
        lba0[446 + 4] == MBR_GPT_TYPE
    }

    /// Rewrites Protective MBR at LBA 0
    pub fn repair_protective_mbr(&self, lba0: &mut [u8]) -> Result<(), String> {
        if lba0.len() < SECTOR_SIZE {
            return Err("Buffer smaller than 512 bytes".to_string());
        }
        lba0.fill(0);

        let p1 = 446;
        lba0[p1 + 4] = MBR_GPT_TYPE; // 0xEE
        lba0[p1 + 5] = 0xFF;        // CHS end
        lba0[p1 + 6] = 0xFF;
        lba0[p1 + 7] = 0xFF;

        // Starting LBA = 1
        lba0[p1 + 8..p1 + 12].copy_from_slice(&1u32.to_le_bytes());

        // Size in LBA sectors
        let size = (self.total_sectors.saturating_sub(1)).min(0xFFFF_FFFF) as u32;
        lba0[p1 + 12..p1 + 16].copy_from_slice(&size.to_le_bytes());

        // Boot Signature
        lba0[510] = 0x55;
        lba0[511] = 0xAA;

        Ok(())
    }

    /// Inspects disk sectors and builds a comprehensive health report
    pub fn inspect_disk(
        &self,
        lba0_mbr: &[u8],
        lba1_primary: &[u8],
        lba_backup: &[u8],
    ) -> BootHealthReport {
        let mbr_valid = self.validate_mbr(lba0_mbr);

        let primary_hdr = ParsedGptHeader::parse_sector(lba1_primary);
        let primary_valid = primary_hdr
            .as_ref()
            .map(|h| h.current_lba == 1 && h.is_crc_valid(lba1_primary))
            .unwrap_or(false);

        let backup_lba_expected = self.total_sectors.saturating_sub(1);
        let backup_hdr = ParsedGptHeader::parse_sector(lba_backup);
        let backup_valid = backup_hdr
            .as_ref()
            .map(|h| h.current_lba == backup_lba_expected && h.is_crc_valid(lba_backup))
            .unwrap_or(false);

        let partitions = primary_hdr
            .map(|h| h.num_partition_entries as usize)
            .or_else(|| backup_hdr.map(|h| h.num_partition_entries as usize))
            .unwrap_or(0);

        let recommended_action = if mbr_valid && primary_valid && backup_valid {
            "All boot structures healthy. No action required.".to_string()
        } else if !primary_valid && backup_valid {
            "Restore Primary GPT Header from Backup GPT at end of disk.".to_string()
        } else if primary_valid && !backup_valid {
            "Restore Backup GPT Header from Primary GPT.".to_string()
        } else if !mbr_valid {
            "Repair Protective MBR at LBA 0.".to_string()
        } else {
            "Critical corruption: Both Primary and Backup GPT headers damaged.".to_string()
        };

        BootHealthReport {
            mbr_valid,
            primary_gpt_valid: primary_valid,
            backup_gpt_valid: backup_valid,
            partitions_found: partitions,
            recommended_action,
        }
    }

    /// Restores a corrupted Primary GPT Header from the healthy Backup GPT Header
    pub fn repair_primary_from_backup(
        &self,
        primary_sector: &mut [u8],
        backup_sector: &[u8],
    ) -> Result<(), String> {
        let backup_hdr = ParsedGptHeader::parse_sector(backup_sector)
            .ok_or_else(|| "Backup GPT Header is corrupted or unreadable".to_string())?;

        let backup_lba = self.total_sectors.saturating_sub(1);
        if !backup_hdr.is_crc_valid(backup_sector) {
            return Err("Backup GPT Header CRC32 is invalid".to_string());
        }

        // Copy backup header to primary
        primary_sector[..512].copy_from_slice(&backup_sector[..512]);

        // Swap LBAs: Primary current LBA = 1, backup LBA = backup_lba
        primary_sector[24..32].copy_from_slice(&1u64.to_le_bytes());
        primary_sector[32..40].copy_from_slice(&backup_lba.to_le_bytes());
        // Primary partition array sits at LBA 2
        primary_sector[72..80].copy_from_slice(&2u64.to_le_bytes());

        // Zero out old CRC and recompute
        primary_sector[16..20].copy_from_slice(&[0, 0, 0, 0]);
        let new_crc = crc32(&primary_sector[..backup_hdr.header_size as usize]);
        primary_sector[16..20].copy_from_slice(&new_crc.to_le_bytes());

        // Validate restored header
        let restored_hdr = ParsedGptHeader::parse_sector(primary_sector)
            .ok_or_else(|| "Restored Primary GPT Header could not be parsed".to_string())?;

        if !restored_hdr.is_crc_valid(primary_sector) || restored_hdr.current_lba != 1 {
            return Err("Restored Primary GPT failed post-repair validation".to_string());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protective_mbr_repair_and_validation() {
        let repairer = BootRepairManager::new(20971520);
        let mut mbr = [0u8; 512];
        assert!(!repairer.validate_mbr(&mbr));

        assert!(repairer.repair_protective_mbr(&mut mbr).is_ok());
        assert!(repairer.validate_mbr(&mbr));
    }

    #[test]
    fn test_primary_gpt_restoration_from_backup() {
        let total_sectors = 20971520u64;
        let backup_lba = total_sectors - 1;
        let repairer = BootRepairManager::new(total_sectors);

        // Synthesize valid backup header
        let mut backup = vec![0u8; 512];
        backup[0..8].copy_from_slice(&GPT_HEADER_MAGIC.to_le_bytes());
        backup[8..12].copy_from_slice(&0x00010000u32.to_le_bytes()); // Rev 1.0
        backup[12..16].copy_from_slice(&92u32.to_le_bytes());        // Size 92
        backup[24..32].copy_from_slice(&backup_lba.to_le_bytes());   // Current LBA
        backup[32..40].copy_from_slice(&1u64.to_le_bytes());         // Backup LBA
        backup[40..48].copy_from_slice(&34u64.to_le_bytes());        // First usable
        backup[48..56].copy_from_slice(&(backup_lba - 34).to_le_bytes());
        backup[80..84].copy_from_slice(&128u32.to_le_bytes());
        backup[84..88].copy_from_slice(&128u32.to_le_bytes());

        let bcrc = crc32(&backup[..92]);
        backup[16..20].copy_from_slice(&bcrc.to_le_bytes());

        // Create wiped/corrupt primary
        let mut primary = vec![0u8; 512];
        let report = repairer.inspect_disk(&[0u8; 512], &primary, &backup);
        assert!(!report.primary_gpt_valid);
        assert!(report.backup_gpt_valid);

        // Perform repair
        assert!(repairer.repair_primary_from_backup(&mut primary, &backup).is_ok());

        let report2 = repairer.inspect_disk(&[0u8; 512], &primary, &backup);
        assert!(report2.primary_gpt_valid);
    }
}
