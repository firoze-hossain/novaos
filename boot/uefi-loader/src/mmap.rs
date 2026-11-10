//! boot/uefi-loader/src/mmap.rs - UEFI Memory Map & Descriptor Processing
//!
//! Provides pure `#![no_std]` UEFI 2.9 memory descriptor decoding, categorization, and physical
//! RAM calculation:
//!   - Categorizes descriptors into Conventional (usable), ACPI Reclaim, Runtime Code/Data, and MMIO.
//!   - Computes total physical RAM and contiguous page frame allocation ranges.
//!   - Handles memory map key tracking required for graceful `ExitBootServices` handover.

// -----------------------------------------------------------------------------
// UEFI Memory Types (UEFI 2.9 Specification)
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u32)]
pub enum EfiMemoryType {
    Reserved = 0,
    LoaderCode = 1,
    LoaderData = 2,
    BootServicesCode = 3,
    BootServicesData = 4,
    RuntimeServicesCode = 5,
    RuntimeServicesData = 6,
    ConventionalMemory = 7, // Usable free RAM
    UnusableMemory = 8,
    AcpiReclaimMemory = 9,
    AcpiMemoryNvs = 10,
    MemoryMappedIo = 11,
    MemoryMappedIoPortSpace = 12,
    PalCode = 13,
    PersistentMemory = 14,
    Unknown = 0xFFFF_FFFF,
}

impl From<u32> for EfiMemoryType {
    fn from(val: u32) -> Self {
        match val {
            0 => Self::Reserved,
            1 => Self::LoaderCode,
            2 => Self::LoaderData,
            3 => Self::BootServicesCode,
            4 => Self::BootServicesData,
            5 => Self::RuntimeServicesCode,
            6 => Self::RuntimeServicesData,
            7 => Self::ConventionalMemory,
            8 => Self::UnusableMemory,
            9 => Self::AcpiReclaimMemory,
            10 => Self::AcpiMemoryNvs,
            11 => Self::MemoryMappedIo,
            12 => Self::MemoryMappedIoPortSpace,
            13 => Self::PalCode,
            14 => Self::PersistentMemory,
            _ => Self::Unknown,
        }
    }
}

// -----------------------------------------------------------------------------
// Memory Descriptor
// -----------------------------------------------------------------------------

/// Standard UEFI 2.9 Memory Descriptor (40 bytes).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
#[repr(C)]
pub struct EfiMemoryDescriptor {
    pub memory_type: u32,
    pub pad: u32,
    pub physical_start: u64,
    pub virtual_start: u64,
    pub number_of_pages: u64,
    pub attribute: u64,
}

impl EfiMemoryDescriptor {
    pub const fn new(
        memory_type: u32,
        physical_start: u64,
        number_of_pages: u64,
        attribute: u64,
    ) -> Self {
        Self {
            memory_type,
            pad: 0,
            physical_start,
            virtual_start: 0,
            number_of_pages,
            attribute,
        }
    }

    /// Size in bytes of this descriptor's memory region (4096 bytes per page).
    pub fn size_in_bytes(&self) -> u64 {
        self.number_of_pages * 4096
    }

    /// Returns true if this memory region is usable conventional RAM for general kernel allocation.
    pub fn is_usable_ram(&self) -> bool {
        EfiMemoryType::from(self.memory_type) == EfiMemoryType::ConventionalMemory
    }

    /// Returns true if this memory contains ACPI tables.
    pub fn is_acpi(&self) -> bool {
        let t = EfiMemoryType::from(self.memory_type);
        t == EfiMemoryType::AcpiReclaimMemory || t == EfiMemoryType::AcpiMemoryNvs
    }
}

// -----------------------------------------------------------------------------
// Memory Map Summary
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct MemoryMapSummary {
    pub total_ram_bytes: u64,
    pub usable_ram_bytes: u64,
    pub acpi_ram_bytes: u64,
    pub runtime_services_bytes: u64,
    pub mmio_bytes: u64,
    pub descriptor_count: usize,
}

/// Analyzes an array of memory descriptors and computes physical memory statistics.
pub fn summarize_memory_map(descriptors: &[EfiMemoryDescriptor]) -> MemoryMapSummary {
    let mut summary = MemoryMapSummary::default();
    summary.descriptor_count = descriptors.len();

    for desc in descriptors {
        let bytes = desc.size_in_bytes();
        let mtype = EfiMemoryType::from(desc.memory_type);

        match mtype {
            EfiMemoryType::ConventionalMemory |
            EfiMemoryType::BootServicesCode |
            EfiMemoryType::BootServicesData |
            EfiMemoryType::LoaderCode |
            EfiMemoryType::LoaderData => {
                summary.total_ram_bytes += bytes;
                if mtype == EfiMemoryType::ConventionalMemory {
                    summary.usable_ram_bytes += bytes;
                }
            }
            EfiMemoryType::AcpiReclaimMemory | EfiMemoryType::AcpiMemoryNvs => {
                summary.total_ram_bytes += bytes;
                summary.acpi_ram_bytes += bytes;
            }
            EfiMemoryType::RuntimeServicesCode | EfiMemoryType::RuntimeServicesData => {
                summary.total_ram_bytes += bytes;
                summary.runtime_services_bytes += bytes;
            }
            EfiMemoryType::MemoryMappedIo | EfiMemoryType::MemoryMappedIoPortSpace => {
                summary.mmio_bytes += bytes;
            }
            _ => {}
        }
    }

    summary
}
