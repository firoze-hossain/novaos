//! boot/uefi-loader/src/lib.rs - Pure Rust Modern UEFI 2.9 Bootloader Engine
//!
//! Provides the complete UEFI bootloader subsystem for NovaOS 2.0 (x86_64 Long Mode):
//!   - GUID Partition Table (GPT) & Protective MBR parsing.
//!   - Secure Boot PE/COFF Authenticode signature checking.
//!   - UEFI Graphics Output Protocol (GOP) mode query and native 1080p/4K linear framebuffer.
//!   - 64-bit ELF kernel image loader with higher-half mapping.
//!   - Physical memory map descriptor processing.
//!   - Modern 64-bit `BootInfo` handover contract.

// Standalone / module compatibility: `#![no_std]` is set at crate root (kernel/rust/lib.rs).

pub mod gpt;
pub mod pe;
pub mod gop;
pub mod elf;
pub mod mmap;
pub mod boot_info;

pub use gpt::{
    crc32, validate_protective_mbr, GptError, GptHeader, GptPartitionEntry, GptPartitionTable,
    Guid, MbrValidation, GUID_BASIC_DATA, GUID_EFI_SYSTEM_PARTITION, GUID_ROOT_FILESYSTEM,
};
pub use pe::{
    verify_secure_boot_image, Pe32PlusHeader, PeError, SecureBootPolicy, SecureBootStatus,
    Sha256,
};
pub use gop::{
    score_gop_mode, select_best_mode, GopFramebuffer, GopModeInfo, GopPixelFormat,
};
pub use elf::{
    plan_kernel_load, Elf64Header, Elf64ProgramHeader, ElfError, KernelLoadPlan,
    KERNEL_HIGHER_HALF_BASE,
};
pub use mmap::{
    summarize_memory_map, EfiMemoryDescriptor, EfiMemoryType, MemoryMapSummary,
};
pub use boot_info::{
    BootInfo, BOOT_INFO_MAGIC, BOOT_INFO_VERSION, KERNEL_VIRTUAL_BASE,
};

// -----------------------------------------------------------------------------
// Comprehensive Self-Test & Verification Engine
// -----------------------------------------------------------------------------

/// Runs the complete architectural self-test suite for the UEFI bootloader and GPT engine.
pub fn run_uefi_boot_selftest() -> Result<(), &'static str> {
    // -------------------------------------------------------------------------
    // 1. Protective MBR Validation
    // -------------------------------------------------------------------------
    let mut mbr = [0u8; 512];
    mbr[510] = 0x55;
    mbr[511] = 0xAA;
    // Set partition 1 type to 0xEE (GPT Protective)
    mbr[446 + 4] = gpt::MBR_GPT_PROTECTIVE_TYPE;
    // Set starting LBA to 1
    mbr[446 + 8] = 0x01;
    let mbr_status = validate_protective_mbr(&mbr);
    if mbr_status != MbrValidation::ValidProtectiveGpt {
        return Err("Protective MBR validation failed");
    }

    // -------------------------------------------------------------------------
    // 2. GPT Header & Partition Array CRC32 Verification
    // -------------------------------------------------------------------------
    let mut header_sector = [0u8; 512];
    let sig_bytes = gpt::GPT_HEADER_SIGNATURE.to_le_bytes();
    header_sector[0..8].copy_from_slice(&sig_bytes);
    header_sector[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes()); // Revision 1.0 (UEFI 2.x)
    header_sector[12..16].copy_from_slice(&92u32.to_le_bytes()); // Header size 92 bytes
    header_sector[24..32].copy_from_slice(&1u64.to_le_bytes()); // My LBA = 1
    header_sector[32..40].copy_from_slice(&204799u64.to_le_bytes()); // Alternate LBA
    header_sector[40..48].copy_from_slice(&34u64.to_le_bytes()); // First usable LBA
    header_sector[48..56].copy_from_slice(&204766u64.to_le_bytes()); // Last usable LBA

    let disk_guid = Guid::from_fields(0x12345678, 0x1234, 0x5678, [1, 2, 3, 4, 5, 6, 7, 8]);
    header_sector[56..72].copy_from_slice(&disk_guid.to_bytes());
    header_sector[72..80].copy_from_slice(&2u64.to_le_bytes()); // Partition entries LBA = 2
    header_sector[80..84].copy_from_slice(&4u32.to_le_bytes()); // 4 partition entries
    header_sector[84..88].copy_from_slice(&128u32.to_le_bytes()); // Entry size = 128

    // Synthesize partition entries array (4 entries * 128 = 512 bytes)
    let mut entries_bytes = [0u8; 512];

    // Entry 0: EFI System Partition (ESP)
    let esp_bytes = GUID_EFI_SYSTEM_PARTITION.to_bytes();
    entries_bytes[0..16].copy_from_slice(&esp_bytes);
    let esp_uid = Guid::from_fields(0xAAAAAAAA, 0xBBBB, 0xCCCC, [1, 1, 1, 1, 1, 1, 1, 1]).to_bytes();
    entries_bytes[16..32].copy_from_slice(&esp_uid);
    entries_bytes[32..40].copy_from_slice(&2048u64.to_le_bytes()); // Start LBA = 2048
    entries_bytes[40..48].copy_from_slice(&102399u64.to_le_bytes()); // End LBA = 102399

    // Entry 1: NovaOS / Linux Root Filesystem Partition
    let root_bytes = GUID_ROOT_FILESYSTEM.to_bytes();
    entries_bytes[128..144].copy_from_slice(&root_bytes);
    let root_uid = Guid::from_fields(0xDDDDDDDD, 0xEEEE, 0xFFFF, [2, 2, 2, 2, 2, 2, 2, 2]).to_bytes();
    entries_bytes[144..160].copy_from_slice(&root_uid);
    entries_bytes[160..168].copy_from_slice(&102400u64.to_le_bytes()); // Start LBA = 102400
    entries_bytes[168..176].copy_from_slice(&204766u64.to_le_bytes()); // End LBA = 204766

    // Compute partition entry array CRC32
    let array_crc = crc32(&entries_bytes);
    header_sector[88..92].copy_from_slice(&array_crc.to_le_bytes());

    // Compute header CRC32 with CRC field zeroed
    header_sector[16..20].copy_from_slice(&[0, 0, 0, 0]);
    let header_crc = crc32(&header_sector[..92]);
    header_sector[16..20].copy_from_slice(&header_crc.to_le_bytes());

    // Parse GPT Table
    let table = GptPartitionTable::parse(&header_sector, &entries_bytes)
        .map_err(|_| "Failed to parse synthesized GPT partition table")?;

    if table.entry_count < 2 {
        return Err("Expected at least 2 active partition entries");
    }

    let esp_part = table.find_esp_partition().ok_or("Failed to discover ESP partition")?;
    if esp_part.starting_lba != 2048 {
        return Err("ESP partition starting LBA mismatch");
    }

    let root_part = table.find_root_partition().ok_or("Failed to discover Root partition")?;
    if root_part.starting_lba != 102400 {
        return Err("Root partition starting LBA mismatch");
    }

    // -------------------------------------------------------------------------
    // 3. Secure Boot PE/COFF Header & Authenticode Digest
    // -------------------------------------------------------------------------
    let mut pe_image = [0u8; 512];
    // DOS Header: "MZ" at 0, e_lfanew = 128 at offset 60
    pe_image[0] = b'M';
    pe_image[1] = b'Z';
    pe_image[60..64].copy_from_slice(&128u32.to_le_bytes());

    // PE Signature at 128: "PE\0\0"
    pe_image[128..132].copy_from_slice(&[b'P', b'E', 0, 0]);
    // COFF Header at 132: Machine = 0x8664 (x86_64)
    pe_image[132..134].copy_from_slice(&pe::MACHINE_X86_64.to_le_bytes());
    pe_image[134..136].copy_from_slice(&1u16.to_le_bytes()); // Number of sections = 1
    pe_image[148..150].copy_from_slice(&240u16.to_le_bytes()); // Size of optional header = 240

    // Optional Header 64-bit at 152: Magic = 0x020B (PE32+)
    pe_image[152..154].copy_from_slice(&pe::PE32_PLUS_MAGIC.to_le_bytes());
    pe_image[168..172].copy_from_slice(&0x1000u32.to_le_bytes()); // Entry point RVA = 0x1000
    pe_image[176..184].copy_from_slice(&0x0040_0000u64.to_le_bytes()); // Image base
    pe_image[212..216].copy_from_slice(&512u32.to_le_bytes()); // Size of headers = 512
    pe_image[220..222].copy_from_slice(&pe::SUBSYSTEM_EFI_APPLICATION.to_le_bytes()); // Subsystem = EFI

    // Checksum offset is 152 + 64 = 216
    pe_image[216..220].copy_from_slice(&0x12345678u32.to_le_bytes());

    // Security Directory at 152 + 144 = 296
    pe_image[296..300].copy_from_slice(&500u32.to_le_bytes()); // VirtualAddress = 500
    pe_image[300..304].copy_from_slice(&12u32.to_le_bytes()); // Size = 12

    let parsed_pe = Pe32PlusHeader::parse(&pe_image)
        .map_err(|_| "Failed to parse synthesized PE32+ header")?;

    let auth_hash = parsed_pe.compute_authenticode_hash(&pe_image);
    if auth_hash == [0u8; 32] {
        return Err("Authenticode hash resulted in zero digest");
    }

    let sb_status = verify_secure_boot_image(&pe_image, SecureBootPolicy::AuditOnly, None);
    match sb_status {
        SecureBootStatus::Verified { .. } => {}
        _ => return Err("Secure Boot verification failed under Audit policy"),
    }

    // -------------------------------------------------------------------------
    // 4. GOP Display Mode Query & Selection
    // -------------------------------------------------------------------------
    let modes = [
        GopModeInfo {
            version: 1,
            width: 1024,
            height: 768,
            pixel_format: GopPixelFormat::PixelBlueGreenRedReserved8BitPerColor,
            pixels_per_scan_line: 1024,
        },
        GopModeInfo {
            version: 1,
            width: 1920,
            height: 1080,
            pixel_format: GopPixelFormat::PixelBlueGreenRedReserved8BitPerColor,
            pixels_per_scan_line: 1920,
        },
        GopModeInfo {
            version: 1,
            width: 3840,
            height: 2160,
            pixel_format: GopPixelFormat::PixelBlueGreenRedReserved8BitPerColor,
            pixels_per_scan_line: 3840,
        },
    ];

    let (best_idx, best_mode) = select_best_mode(&modes)
        .ok_or("Failed to select best GOP display mode")?;

    // Should prioritize 4K UHD mode (idx 2)
    if best_idx != 2 || best_mode.width != 3840 {
        return Err("GOP mode selector did not prioritize native 4K display");
    }

    let fb = GopFramebuffer::new(
        0xE000_0000,
        (1920 * 1080 * 4) as u64,
        1920,
        1080,
        1920,
        GopPixelFormat::PixelBlueGreenRedReserved8BitPerColor,
    );
    if !fb.is_high_definition() {
        return Err("1080p framebuffer was not classified as high definition");
    }
    if fb.pixel_byte_offset(100, 100).is_none() {
        return Err("Pixel offset calculation failed for valid coordinate");
    }

    // -------------------------------------------------------------------------
    // 5. 64-Bit ELF Loader Plan & Entry Point Resolution
    // -------------------------------------------------------------------------
    let mut elf_data = [0u8; 256];
    elf_data[0..4].copy_from_slice(&elf::ELF_MAGIC);
    elf_data[4] = elf::ELF_CLASS_64;
    elf_data[5] = elf::ELF_DATA_2LSB;
    elf_data[16..18].copy_from_slice(&elf::ET_EXEC.to_le_bytes());
    elf_data[18..20].copy_from_slice(&elf::EM_X86_64.to_le_bytes());
    elf_data[24..32].copy_from_slice(&0xFFFF_8000_0010_0000u64.to_le_bytes()); // Higher-half entry point
    elf_data[32..40].copy_from_slice(&64u64.to_le_bytes()); // Program headers offset at 64
    elf_data[54..56].copy_from_slice(&56u16.to_le_bytes()); // Program header entry size = 56
    elf_data[56..58].copy_from_slice(&1u16.to_le_bytes()); // 1 program header

    // Synthesize Program Header at offset 64
    elf_data[64..68].copy_from_slice(&elf::PT_LOAD.to_le_bytes()); // PT_LOAD
    elf_data[68..72].copy_from_slice(&(elf::PF_R | elf::PF_X).to_le_bytes()); // Read + Execute
    elf_data[72..80].copy_from_slice(&0u64.to_le_bytes()); // Offset 0
    elf_data[80..88].copy_from_slice(&0xFFFF_8000_0010_0000u64.to_le_bytes()); // vaddr
    elf_data[88..96].copy_from_slice(&0x0010_0000u64.to_le_bytes()); // paddr
    elf_data[96..104].copy_from_slice(&256u64.to_le_bytes()); // filesz
    elf_data[104..112].copy_from_slice(&256u64.to_le_bytes()); // memsz

    let plan = plan_kernel_load(&elf_data, 0x0010_0000)
        .map_err(|_| "Failed to plan 64-bit ELF kernel load")?;

    if plan.entry_point != 0xFFFF_8000_0010_0000 {
        return Err("Kernel entry point resolution mismatch");
    }
    if plan.segment_count != 1 {
        return Err("Kernel segment count mismatch");
    }

    // -------------------------------------------------------------------------
    // 6. Memory Map & BootInfo Contract Handover
    // -------------------------------------------------------------------------
    let descriptors = [
        EfiMemoryDescriptor::new(EfiMemoryType::ConventionalMemory as u32, 0x100_0000, 65536, 0xF),
        EfiMemoryDescriptor::new(EfiMemoryType::AcpiReclaimMemory as u32, 0x7FE_0000, 32, 0xF),
        EfiMemoryDescriptor::new(EfiMemoryType::RuntimeServicesData as u32, 0x7FE_2000, 16, 0xF),
    ];
    let mmap_summary = summarize_memory_map(&descriptors);
    if mmap_summary.usable_ram_bytes < (256 * 1024 * 1024) {
        return Err("Memory map usable RAM underflow");
    }

    let boot_info = BootInfo::new(
        &fb,
        &mmap_summary,
        0x1000,
        descriptors.len() as u64,
        40,
        0x7FE0000,
        0x100000,
        plan.entry_point,
        true,
        "console=tty0 root=UUID=0fc63daf-8483-4772-8e79-3d69d8477de4",
    );
    boot_info.validate()?;

    Ok(())
}

// -----------------------------------------------------------------------------
// C FFI Export for Kernel Self-Test & Diagnostic Callers
// -----------------------------------------------------------------------------

/// Exported C FFI function for kernel boot self-test and verification.
/// Returns 0 on success, or a negative error code on failure.
#[no_mangle]
pub extern "C" fn rust_uefi_boot_selftest() -> i32 {
    match run_uefi_boot_selftest() {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

// -----------------------------------------------------------------------------
// Host Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protective_mbr() {
        let mut mbr = [0u8; 512];
        mbr[510] = 0x55;
        mbr[511] = 0xAA;
        mbr[446 + 4] = gpt::MBR_GPT_PROTECTIVE_TYPE;
        assert_eq!(validate_protective_mbr(&mbr), MbrValidation::ValidProtectiveGpt);

        // Test non-protective MBR
        mbr[446 + 4] = 0x83; // Linux native
        assert_eq!(validate_protective_mbr(&mbr), MbrValidation::LegacyMbrFound);

        // Test invalid signature
        mbr[510] = 0x00;
        assert_eq!(validate_protective_mbr(&mbr), MbrValidation::InvalidSignature);
    }

    #[test]
    fn test_guid_serialization() {
        let guid = GUID_ROOT_FILESYSTEM;
        let bytes = guid.to_bytes();
        let parsed = Guid::from_bytes(&bytes).expect("Failed to parse GUID from bytes");
        assert_eq!(guid, parsed);
        assert!(!guid.is_zero());
    }

    #[test]
    fn test_crc32() {
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
    }

    #[test]
    fn test_gop_mode_scoring() {
        let fhd = GopModeInfo {
            version: 1,
            width: 1920,
            height: 1080,
            pixel_format: GopPixelFormat::PixelBlueGreenRedReserved8BitPerColor,
            pixels_per_scan_line: 1920,
        };
        let uhd = GopModeInfo {
            version: 1,
            width: 3840,
            height: 2160,
            pixel_format: GopPixelFormat::PixelBlueGreenRedReserved8BitPerColor,
            pixels_per_scan_line: 3840,
        };
        assert!(score_gop_mode(&uhd) > score_gop_mode(&fhd));
    }

    #[test]
    fn test_full_uefi_boot_selftest_suite() {
        assert_eq!(run_uefi_boot_selftest(), Ok(()));
        assert_eq!(rust_uefi_boot_selftest(), 0);
    }
}
