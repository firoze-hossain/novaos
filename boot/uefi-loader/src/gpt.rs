//! boot/uefi-loader/src/gpt.rs - GUID Partition Table (GPT) & Protective MBR Engine
//!
//! Provides pure `#![no_std]` GPT discovery and parsing conforming to UEFI 2.9 Specification:
//!   - Protective MBR (LBA 0) validation with partition type `0xEE`.
//!   - Primary GPT Header (LBA 1) signature ("EFI PART" / 0x5452415020494645).
//!   - CRC32 verification of GPT Header and Partition Entry Array.
//!   - GPT Partition Entry (128 bytes) parsing with UTF-16LE partition names.
//!   - Automated discovery of NovaOS / Linux Root Filesystem GUID:
//!       `0FC63DAF-8483-4772-8E79-3D69D8477DE4`
//!   - Automated discovery of EFI System Partition (ESP) GUID:
//!       `C12A7328-F81F-11D2-BA4B-00A0C93EC93B`

/// Standard GPT sector size in bytes.
pub const SECTOR_SIZE: usize = 512;

/// Primary GPT Header magic signature: "EFI PART" (0x5452415020494645 in LE).
pub const GPT_HEADER_SIGNATURE: u64 = 0x5452_4150_2049_4645;

/// Minimum GPT Header size specified in UEFI 2.9 (92 bytes).
pub const GPT_HEADER_MIN_SIZE: u32 = 92;

/// Standard GPT Partition Entry size in bytes.
pub const GPT_ENTRY_SIZE: usize = 128;

/// Protective MBR partition type flag for GPT disks.
pub const MBR_GPT_PROTECTIVE_TYPE: u8 = 0xEE;

/// Standard MBR boot signature (0x55, 0xAA) at offset 510.
pub const MBR_SIGNATURE: u16 = 0xAA55;

// -----------------------------------------------------------------------------
// Well-Known Partition Type GUIDs (Mixed-Endian RFC 4122 / UEFI)
// -----------------------------------------------------------------------------

/// NovaOS / Linux Root Filesystem Partition GUID: `0FC63DAF-8483-4772-8E79-3D69D8477DE4`
pub const GUID_ROOT_FILESYSTEM: Guid = Guid::from_fields(
    0x0FC63DAF,
    0x8483,
    0x4772,
    [0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D, 0xE4],
);

/// EFI System Partition (ESP) GUID: `C12A7328-F81F-11D2-BA4B-00A0C93EC93B`
pub const GUID_EFI_SYSTEM_PARTITION: Guid = Guid::from_fields(
    0xC12A7328,
    0xF81F,
    0x11D2,
    [0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B],
);

/// Microsoft Basic Data Partition GUID: `EBD0A0A2-B9E5-4433-87C0-68B6B72699C7`
pub const GUID_BASIC_DATA: Guid = Guid::from_fields(
    0xEBD0A0A2,
    0xB9E5,
    0x4433,
    [0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7],
);

/// Linux Swap Partition GUID: `0657FD6D-A4AB-43C4-84E5-0933C84B4F4F`
pub const GUID_LINUX_SWAP: Guid = Guid::from_fields(
    0x0657FD6D,
    0xA4AB,
    0x43C4,
    [0x84, 0xE5, 0x09, 0x33, 0xC8, 0x4B, 0x4F, 0x4F],
);

// -----------------------------------------------------------------------------
// Guid Type Definition
// -----------------------------------------------------------------------------

/// 128-bit globally unique identifier in UEFI mixed-endian format.
#[derive(Copy, Clone, PartialEq, Eq, Default)]
#[repr(C)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl core::fmt::Debug for Guid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
            self.data1,
            self.data2,
            self.data3,
            self.data4[0],
            self.data4[1],
            self.data4[2],
            self.data4[3],
            self.data4[4],
            self.data4[5],
            self.data4[6],
            self.data4[7]
        )
    }
}

impl Guid {
    pub const fn from_fields(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Self {
        Self {
            data1,
            data2,
            data3,
            data4,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 16 {
            return None;
        }
        let data1 = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let data2 = u16::from_le_bytes([bytes[4], bytes[5]]);
        let data3 = u16::from_le_bytes([bytes[6], bytes[7]]);
        let mut data4 = [0u8; 8];
        data4.copy_from_slice(&bytes[8..16]);
        Some(Self {
            data1,
            data2,
            data3,
            data4,
        })
    }

    pub fn to_bytes(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        let d1 = self.data1.to_le_bytes();
        let d2 = self.data2.to_le_bytes();
        let d3 = self.data3.to_le_bytes();
        out[0..4].copy_from_slice(&d1);
        out[4..6].copy_from_slice(&d2);
        out[6..8].copy_from_slice(&d3);
        out[8..16].copy_from_slice(&self.data4);
        out
    }

    pub fn is_zero(&self) -> bool {
        self.data1 == 0 && self.data2 == 0 && self.data3 == 0 && self.data4 == [0u8; 8]
    }
}

// -----------------------------------------------------------------------------
// CRC32 Implementation (IEEE 802.3 standard)
// -----------------------------------------------------------------------------

/// Computes standard IEEE 802.3 CRC-32 used throughout UEFI GPT headers.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = if (crc & 1) != 0 { 0xEDB8_8320 } else { 0 };
            crc = (crc >> 1) ^ mask;
        }
    }
    !crc
}

// -----------------------------------------------------------------------------
// Protective MBR Structures
// -----------------------------------------------------------------------------

/// Legacy MBR partition table record (16 bytes).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
#[repr(C, packed)]
pub struct MbrPartitionRecord {
    pub boot_indicator: u8,
    pub starting_chs: [u8; 3],
    pub os_type: u8,
    pub ending_chs: [u8; 3],
    pub starting_lba: u32,
    pub size_in_lba: u32,
}

/// Protective MBR validation result.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum MbrValidation {
    ValidProtectiveGpt,
    LegacyMbrFound,
    InvalidSignature,
    InvalidRecord,
}

/// Validates protective MBR at LBA 0 (512 bytes).
pub fn validate_protective_mbr(sector: &[u8]) -> MbrValidation {
    if sector.len() < SECTOR_SIZE {
        return MbrValidation::InvalidRecord;
    }
    let sig = u16::from_le_bytes([sector[510], sector[511]]);
    if sig != MBR_SIGNATURE {
        return MbrValidation::InvalidSignature;
    }

    // Inspect first partition record at offset 446 (0x1BE)
    let os_type = sector[446 + 4];
    if os_type == MBR_GPT_PROTECTIVE_TYPE {
        MbrValidation::ValidProtectiveGpt
    } else {
        MbrValidation::LegacyMbrFound
    }
}

// -----------------------------------------------------------------------------
// Primary GPT Header Structure
// -----------------------------------------------------------------------------

/// UEFI 2.9 Primary GPT Header (92 bytes, located at LBA 1).
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct GptHeader {
    pub signature: u64,
    pub revision: u32,
    pub header_size: u32,
    pub header_crc32: u32,
    pub reserved: u32,
    pub my_lba: u64,
    pub alternate_lba: u64,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
    pub disk_guid: Guid,
    pub partition_entry_lba: u64,
    pub num_partition_entries: u32,
    pub size_of_partition_entry: u32,
    pub partition_entry_array_crc32: u32,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum GptError {
    BufferTooShort,
    InvalidSignature,
    UnsupportedRevision,
    InvalidHeaderSize,
    HeaderCrcMismatch { expected: u32, calculated: u32 },
    PartitionArrayCrcMismatch { expected: u32, calculated: u32 },
    InvalidEntrySize,
}

impl GptHeader {
    /// Parses and verifies a GPT header from sector bytes (LBA 1).
    pub fn parse_and_validate(sector: &[u8]) -> Result<Self, GptError> {
        if sector.len() < GPT_HEADER_MIN_SIZE as usize {
            return Err(GptError::BufferTooShort);
        }

        let signature = u64::from_le_bytes(sector[0..8].try_into().unwrap());
        if signature != GPT_HEADER_SIGNATURE {
            return Err(GptError::InvalidSignature);
        }

        let revision = u32::from_le_bytes(sector[8..12].try_into().unwrap());
        let header_size = u32::from_le_bytes(sector[12..16].try_into().unwrap());
        if header_size < GPT_HEADER_MIN_SIZE || (header_size as usize) > sector.len() {
            return Err(GptError::InvalidHeaderSize);
        }

        let header_crc32 = u32::from_le_bytes(sector[16..20].try_into().unwrap());
        let reserved = u32::from_le_bytes(sector[20..24].try_into().unwrap());
        let my_lba = u64::from_le_bytes(sector[24..32].try_into().unwrap());
        let alternate_lba = u64::from_le_bytes(sector[32..40].try_into().unwrap());
        let first_usable_lba = u64::from_le_bytes(sector[40..48].try_into().unwrap());
        let last_usable_lba = u64::from_le_bytes(sector[48..56].try_into().unwrap());

        let disk_guid = Guid::from_bytes(&sector[56..72]).ok_or(GptError::BufferTooShort)?;
        let partition_entry_lba = u64::from_le_bytes(sector[72..80].try_into().unwrap());
        let num_partition_entries = u32::from_le_bytes(sector[80..84].try_into().unwrap());
        let size_of_partition_entry = u32::from_le_bytes(sector[84..88].try_into().unwrap());
        let partition_entry_array_crc32 = u32::from_le_bytes(sector[88..92].try_into().unwrap());

        // Validate Header CRC32: compute with CRC32 field zeroed
        let mut header_buf = [0u8; 128];
        let copy_len = header_size as usize;
        header_buf[..copy_len].copy_from_slice(&sector[..copy_len]);
        header_buf[16..20].copy_from_slice(&[0, 0, 0, 0]);

        let calculated_crc = crc32(&header_buf[..copy_len]);
        if calculated_crc != header_crc32 {
            return Err(GptError::HeaderCrcMismatch {
                expected: header_crc32,
                calculated: calculated_crc,
            });
        }

        Ok(Self {
            signature,
            revision,
            header_size,
            header_crc32,
            reserved,
            my_lba,
            alternate_lba,
            first_usable_lba,
            last_usable_lba,
            disk_guid,
            partition_entry_lba,
            num_partition_entries,
            size_of_partition_entry,
            partition_entry_array_crc32,
        })
    }
}

// -----------------------------------------------------------------------------
// GPT Partition Entry Structure
// -----------------------------------------------------------------------------

/// Individual GPT partition entry (128 bytes).
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct GptPartitionEntry {
    pub partition_type_guid: Guid,
    pub unique_partition_guid: Guid,
    pub starting_lba: u64,
    pub ending_lba: u64,
    pub attributes: u64,
    pub name: [u16; 36],
}

impl Default for GptPartitionEntry {
    fn default() -> Self {
        Self {
            partition_type_guid: Guid::default(),
            unique_partition_guid: Guid::default(),
            starting_lba: 0,
            ending_lba: 0,
            attributes: 0,
            name: [0u16; 36],
        }
    }
}

impl GptPartitionEntry {
    /// Parses an entry from a 128-byte slice.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < GPT_ENTRY_SIZE {
            return None;
        }
        let partition_type_guid = Guid::from_bytes(&bytes[0..16])?;
        let unique_partition_guid = Guid::from_bytes(&bytes[16..32])?;
        let starting_lba = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
        let ending_lba = u64::from_le_bytes(bytes[40..48].try_into().unwrap());
        let attributes = u64::from_le_bytes(bytes[48..56].try_into().unwrap());

        let mut name = [0u16; 36];
        for i in 0..36 {
            let offset = 56 + i * 2;
            name[i] = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        }

        Some(Self {
            partition_type_guid,
            unique_partition_guid,
            starting_lba,
            ending_lba,
            attributes,
            name,
        })
    }

    /// Returns true if this partition entry is in active use (non-zero type GUID).
    pub fn is_used(&self) -> bool {
        !self.partition_type_guid.is_zero()
    }

    /// Returns true if this partition is the NovaOS / Linux Root Filesystem.
    pub fn is_root_partition(&self) -> bool {
        self.partition_type_guid == GUID_ROOT_FILESYSTEM
    }

    /// Returns true if this partition is the EFI System Partition (ESP).
    pub fn is_esp_partition(&self) -> bool {
        self.partition_type_guid == GUID_EFI_SYSTEM_PARTITION
    }

    /// Returns the sector span (size in LBAs) of this partition.
    pub fn sector_count(&self) -> u64 {
        if self.ending_lba >= self.starting_lba {
            self.ending_lba - self.starting_lba + 1
        } else {
            0
        }
    }

    /// Formats the UTF-16LE partition name as ASCII into a provided buffer.
    pub fn get_name_ascii(&self, out: &mut [u8]) -> usize {
        let mut len = 0;
        for &code in &self.name {
            if code == 0 || len >= out.len() {
                break;
            }
            out[len] = if code < 128 { code as u8 } else { b'?' };
            len += 1;
        }
        len
    }
}

// -----------------------------------------------------------------------------
// GPT Table Parser & Discovery Engine
// -----------------------------------------------------------------------------

/// Maximum partition entries parsed in fixed memory footprint.
pub const MAX_PARSED_ENTRIES: usize = 32;

/// Parsed GPT partition table descriptor.
#[derive(Debug, Copy, Clone)]
pub struct GptPartitionTable {
    pub header: GptHeader,
    pub entries: [GptPartitionEntry; MAX_PARSED_ENTRIES],
    pub entry_count: usize,
}

impl GptPartitionTable {
    /// Parses GPT partition table from header sector and entries buffer.
    pub fn parse(header_sector: &[u8], entries_data: &[u8]) -> Result<Self, GptError> {
        let header = GptHeader::parse_and_validate(header_sector)?;

        if header.size_of_partition_entry as usize != GPT_ENTRY_SIZE {
            return Err(GptError::InvalidEntrySize);
        }

        let total_bytes = (header.num_partition_entries as usize) * GPT_ENTRY_SIZE;
        if entries_data.len() < total_bytes {
            return Err(GptError::BufferTooShort);
        }

        let calculated_array_crc = crc32(&entries_data[..total_bytes]);
        if calculated_array_crc != header.partition_entry_array_crc32 {
            return Err(GptError::PartitionArrayCrcMismatch {
                expected: header.partition_entry_array_crc32,
                calculated: calculated_array_crc,
            });
        }

        let mut entries = [GptPartitionEntry::default(); MAX_PARSED_ENTRIES];
        let mut entry_count = 0;
        let limit = core::cmp::min(header.num_partition_entries as usize, MAX_PARSED_ENTRIES);

        for i in 0..limit {
            let offset = i * GPT_ENTRY_SIZE;
            if let Some(entry) = GptPartitionEntry::parse(&entries_data[offset..offset + GPT_ENTRY_SIZE]) {
                if entry.is_used() {
                    entries[entry_count] = entry;
                    entry_count += 1;
                }
            }
        }

        Ok(Self {
            header,
            entries,
            entry_count,
        })
    }

    /// Discovers the NovaOS / Linux Root Filesystem partition.
    pub fn find_root_partition(&self) -> Option<&GptPartitionEntry> {
        for i in 0..self.entry_count {
            if self.entries[i].is_root_partition() {
                return Some(&self.entries[i]);
            }
        }
        None
    }

    /// Discovers the EFI System Partition (ESP).
    pub fn find_esp_partition(&self) -> Option<&GptPartitionEntry> {
        for i in 0..self.entry_count {
            if self.entries[i].is_esp_partition() {
                return Some(&self.entries[i]);
            }
        }
        None
    }
}
