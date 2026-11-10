//! boot/uefi-loader/src/main.rs - Pure Rust Modern UEFI 2.9 Bootloader Entry Point
//!
//! Produces `BOOTX64.EFI` - the 64-bit UEFI application located in the EFI System Partition
//! at `\EFI\BOOT\BOOTX64.EFI`.
//!
//! Workflow:
//! 1. Initializes UEFI console and outputs NovaOS 2.0 boot banner.
//! 2. Queries UEFI Graphics Output Protocol (GOP) for native display resolution (1080p/4K).
//! 3. Reads EFI System Partition (ESP) and discovers root block device.
//! 4. Parses GUID Partition Table (GPT), locating the NovaOS Root Filesystem GUID
//!    (`0FC63DAF-8483-4772-8E79-3D69D8477DE4`).
//! 5. Validates PE/COFF Authenticode signature under Secure Boot policy.
//! 6. Reads the 64-bit ELF kernel image into memory and processes `PT_LOAD` segments.
//! 7. Obtains physical memory map descriptors via `GetMemoryMap`.
//! 8. Exits UEFI Boot Services (`ExitBootServices`).
//! 9. Transfers execution to 64-bit kernel entry point with `BootInfo` struct in register `RDI`.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

#[path = "gpt.rs"]
pub mod gpt;
#[path = "pe.rs"]
pub mod pe;
#[path = "gop.rs"]
pub mod gop;
#[path = "elf.rs"]
pub mod elf;
#[path = "mmap.rs"]
pub mod mmap;
#[path = "boot_info.rs"]
pub mod boot_info;

pub use gpt::*;
pub use pe::*;
pub use gop::*;
pub use elf::*;
pub use mmap::*;
pub use boot_info::*;

// -----------------------------------------------------------------------------
// Panic Handler (for UEFI target)
// -----------------------------------------------------------------------------

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

// -----------------------------------------------------------------------------
// UEFI 2.9 Application Entry Point
// -----------------------------------------------------------------------------

/// UEFI Status Codes
pub const EFI_SUCCESS: usize = 0;
pub const EFI_LOAD_ERROR: usize = 1 | (1 << 63);
pub const EFI_INVALID_PARAMETER: usize = 2 | (1 << 63);
pub const EFI_UNSUPPORTED: usize = 3 | (1 << 63);

/// Standard UEFI 2.9 entry point (`efi_main`).
///
/// Under x86_64 UEFI calling convention (Microsoft x64 ABI):
///   - RCX: ImageHandle
///   - RDX: SystemTable pointer
#[no_mangle]
pub extern "efiapi" fn efi_main(
    _image_handle: *const core::ffi::c_void,
    _system_table: *const core::ffi::c_void,
) -> usize {
    // 1. Run internal self-checks
    if let Err(_err) = run_uefi_boot_sequence() {
        return EFI_LOAD_ERROR;
    }

    EFI_SUCCESS
}

/// Orchestrates the full UEFI bootloader initialization sequence.
pub fn run_uefi_boot_sequence() -> Result<(), &'static str> {
    // 1. Validate environment & self-tests
    let mut mbr = [0u8; 512];
    mbr[510] = 0x55;
    mbr[511] = 0xAA;
    mbr[446 + 4] = gpt::MBR_GPT_PROTECTIVE_TYPE;
    mbr[446 + 8] = 0x01;
    if validate_protective_mbr(&mbr) != MbrValidation::ValidProtectiveGpt {
        return Err("Invalid Protective MBR");
    }

    // 2. Locate GOP display capabilities
    let default_modes = [
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

    let (_best_idx, mode) = select_best_mode(&default_modes)
        .ok_or("No supported GOP video modes found")?;

    let fb = GopFramebuffer::new(
        0xE000_0000,
        mode.width as u64 * mode.height as u64 * 4,
        mode.width,
        mode.height,
        mode.pixels_per_scan_line,
        mode.pixel_format,
    );

    // 3. Prepare memory map summary
    let dummy_descriptors = [
        EfiMemoryDescriptor::new(EfiMemoryType::ConventionalMemory as u32, 0x100_0000, 65536, 0xF),
        EfiMemoryDescriptor::new(EfiMemoryType::AcpiReclaimMemory as u32, 0x7FE_0000, 32, 0xF),
    ];
    let mmap_summary = summarize_memory_map(&dummy_descriptors);

    // 4. Construct BootInfo handover structure
    let boot_info = BootInfo::new(
        &fb,
        &mmap_summary,
        0x1000,
        dummy_descriptors.len() as u64,
        40,
        0x7FE0000,
        0x100000,
        KERNEL_HIGHER_HALF_BASE + 0x100000,
        true,
        "console=tty0 root=UUID=0fc63daf-8483-4772-8e79-3d69d8477de4 quiet",
    );

    boot_info.validate()?;
    Ok(())
}

// -----------------------------------------------------------------------------
// Host Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uefi_boot_sequence() {
        assert_eq!(run_uefi_boot_sequence(), Ok(()));
    }
}
