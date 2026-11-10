//! boot/uefi-loader/src/boot_info.rs - Modern BootInfo Handover Structure
//!
//! Provides the canonical 64-bit `BootInfo` contract passed by the UEFI bootloader to the NovaOS
//! 64-bit Long Mode kernel entry point.

use super::gop::GopFramebuffer;
use super::mmap::MemoryMapSummary;

/// Magic signature for NovaOS UEFI boot info: "NOVAUEFI" in little-endian.
pub const BOOT_INFO_MAGIC: u64 = 0x4E4F_5641_5545_4649;

/// NovaOS BootInfo protocol version: 2.0 (0x0200).
pub const BOOT_INFO_VERSION: u32 = 0x0200;

/// Standard Higher-Half Kernel Base Address in x86_64.
pub const KERNEL_VIRTUAL_BASE: u64 = 0xFFFF_8000_0000_0000;

/// Bootloader handover information structure (passed in register RDI to kernel entry).
#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct BootInfo {
    /// Magic signature (BOOT_INFO_MAGIC = 0x4E4F564155454649).
    pub magic: u64,
    /// Protocol version (0x0200).
    pub version: u32,
    /// Boot type (1 = UEFI 2.9, 0 = BIOS/Multiboot).
    pub boot_type: u32,

    // --- GOP Linear Framebuffer ---
    pub fb_base_phys: u64,
    pub fb_size: u64,
    pub fb_width: u32,
    pub fb_height: u32,
    pub fb_stride: u32,
    pub fb_format: u32, // 1 = BGRX32, 0 = RGBX32

    // --- Physical Memory Map ---
    pub mmap_addr_phys: u64,
    pub mmap_desc_count: u64,
    pub mmap_desc_size: u64,
    pub mmap_desc_version: u32,
    pub total_ram_bytes: u64,
    pub usable_ram_bytes: u64,

    // --- ACPI & Kernel Placement ---
    pub acpi_rsdp_phys: u64,
    pub kernel_phys_base: u64,
    pub kernel_virt_base: u64,
    pub kernel_entry: u64,

    // --- Security & Parameters ---
    pub secure_boot_active: u8,
    pub reserved: [u8; 7],
    pub cmdline: [u8; 256],
}

impl Default for BootInfo {
    fn default() -> Self {
        Self {
            magic: BOOT_INFO_MAGIC,
            version: BOOT_INFO_VERSION,
            boot_type: 1, // UEFI 2.9
            fb_base_phys: 0,
            fb_size: 0,
            fb_width: 0,
            fb_height: 0,
            fb_stride: 0,
            fb_format: 1, // BGRX32 default
            mmap_addr_phys: 0,
            mmap_desc_count: 0,
            mmap_desc_size: 40,
            mmap_desc_version: 1,
            total_ram_bytes: 0,
            usable_ram_bytes: 0,
            acpi_rsdp_phys: 0,
            kernel_phys_base: 0x0010_0000,
            kernel_virt_base: KERNEL_VIRTUAL_BASE,
            kernel_entry: 0,
            secure_boot_active: 0,
            reserved: [0u8; 7],
            cmdline: [0u8; 256],
        }
    }
}

impl BootInfo {
    /// Creates a populated `BootInfo` structure ready for kernel handover.
    pub fn new(
        fb: &GopFramebuffer,
        mmap_summary: &MemoryMapSummary,
        mmap_addr_phys: u64,
        desc_count: u64,
        desc_size: u64,
        acpi_rsdp: u64,
        kernel_phys_base: u64,
        kernel_entry: u64,
        secure_boot: bool,
        cmdline_str: &str,
    ) -> Self {
        let mut info = Self::default();
        info.fb_base_phys = fb.base_phys;
        info.fb_size = fb.size;
        info.fb_width = fb.width;
        info.fb_height = fb.height;
        info.fb_stride = fb.stride;
        info.fb_format = fb.format as u32;

        info.mmap_addr_phys = mmap_addr_phys;
        info.mmap_desc_count = desc_count;
        info.mmap_desc_size = desc_size;
        info.total_ram_bytes = mmap_summary.total_ram_bytes;
        info.usable_ram_bytes = mmap_summary.usable_ram_bytes;

        info.acpi_rsdp_phys = acpi_rsdp;
        info.kernel_phys_base = kernel_phys_base;
        info.kernel_entry = kernel_entry;
        info.secure_boot_active = if secure_boot { 1 } else { 0 };

        let bytes = cmdline_str.as_bytes();
        let copy_len = core::cmp::min(bytes.len(), 255);
        info.cmdline[..copy_len].copy_from_slice(&bytes[..copy_len]);
        info.cmdline[copy_len] = 0;

        info
    }

    /// Validates the BootInfo invariants.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.magic != BOOT_INFO_MAGIC {
            return Err("Invalid BootInfo magic");
        }
        if self.version != BOOT_INFO_VERSION {
            return Err("Unsupported BootInfo version");
        }
        if self.kernel_virt_base < KERNEL_VIRTUAL_BASE {
            return Err("Kernel virtual base is below higher-half boundary");
        }
        if self.fb_width == 0 || self.fb_height == 0 {
            return Err("Invalid GOP framebuffer resolution");
        }
        if self.usable_ram_bytes == 0 {
            return Err("Memory map indicates zero usable RAM");
        }
        Ok(())
    }
}
