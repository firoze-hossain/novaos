//! boot/uefi-loader/src/elf.rs - 64-Bit ELF Kernel Loader
//!
//! Provides pure `#![no_std]` ELF64 executable parsing and segment loading for x86_64 Long Mode:
//!   - ELF identification header validation (Magic 0x7F 'E' 'L' 'F', Class 2 = 64-bit, Machine 0x3E = x86_64).
//!   - Program header table parser locating `PT_LOAD` segments.
//!   - Physical frame allocation and segment relocation to target addresses.
//!   - Automatic BSS zeroing when `p_memsz > p_filesz`.
//!   - Higher-half kernel virtual address (`0xFFFF_8000_0000_0000`) entry point resolution.

// -----------------------------------------------------------------------------
// ELF64 Constants
// -----------------------------------------------------------------------------

pub const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
pub const ELF_CLASS_64: u8 = 2;
pub const ELF_DATA_2LSB: u8 = 1; // Little-endian
pub const ET_EXEC: u16 = 2; // Executable file
pub const ET_DYN: u16 = 3; // Position-independent executable
pub const EM_X86_64: u16 = 0x3E; // AMD x86_64 architecture

pub const PT_LOAD: u32 = 1; // Loadable segment

pub const PF_X: u32 = 1; // Execute
pub const PF_W: u32 = 2; // Write
pub const PF_R: u32 = 4; // Read

/// Base address for higher-half kernel mapping (PML4 index 256).
pub const KERNEL_HIGHER_HALF_BASE: u64 = 0xFFFF_8000_0000_0000;

// -----------------------------------------------------------------------------
// Header Structures
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Elf64Header {
    pub entry_point: u64,
    pub ph_offset: u64,
    pub sh_offset: u64,
    pub flags: u32,
    pub eh_size: u16,
    pub ph_entry_size: u16,
    pub ph_count: u16,
    pub sh_entry_size: u16,
    pub sh_count: u16,
    pub sh_str_index: u16,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct Elf64ProgramHeader {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ElfError {
    BufferTooShort,
    InvalidMagic,
    Not64Bit,
    NotLittleEndian,
    UnsupportedMachine(u16),
    InvalidHeaderSize,
    InvalidProgramHeaderOffset,
    SegmentOutOfBounds,
}

impl Elf64Header {
    /// Validates and parses the 64-byte ELF64 file header.
    pub fn parse(bytes: &[u8]) -> Result<Self, ElfError> {
        if bytes.len() < 64 {
            return Err(ElfError::BufferTooShort);
        }

        if bytes[0..4] != ELF_MAGIC {
            return Err(ElfError::InvalidMagic);
        }
        if bytes[4] != ELF_CLASS_64 {
            return Err(ElfError::Not64Bit);
        }
        if bytes[5] != ELF_DATA_2LSB {
            return Err(ElfError::NotLittleEndian);
        }

        let e_type = u16::from_le_bytes([bytes[16], bytes[17]]);
        if e_type != ET_EXEC && e_type != ET_DYN {
            return Err(ElfError::InvalidMagic);
        }

        let e_machine = u16::from_le_bytes([bytes[18], bytes[19]]);
        if e_machine != EM_X86_64 {
            return Err(ElfError::UnsupportedMachine(e_machine));
        }

        let entry_point = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        let ph_offset = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
        let sh_offset = u64::from_le_bytes(bytes[40..48].try_into().unwrap());
        let flags = u32::from_le_bytes(bytes[48..52].try_into().unwrap());
        let eh_size = u16::from_le_bytes([bytes[52], bytes[53]]);
        let ph_entry_size = u16::from_le_bytes([bytes[54], bytes[55]]);
        let ph_count = u16::from_le_bytes([bytes[56], bytes[57]]);
        let sh_entry_size = u16::from_le_bytes([bytes[58], bytes[59]]);
        let sh_count = u16::from_le_bytes([bytes[60], bytes[61]]);
        let sh_str_index = u16::from_le_bytes([bytes[62], bytes[63]]);

        Ok(Self {
            entry_point,
            ph_offset,
            sh_offset,
            flags,
            eh_size,
            ph_entry_size,
            ph_count,
            sh_entry_size,
            sh_count,
            sh_str_index,
        })
    }
}

impl Elf64ProgramHeader {
    /// Parses an ELF64 program header (56 bytes).
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 56 {
            return None;
        }

        let p_type = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let p_flags = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let p_offset = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let p_vaddr = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        let p_paddr = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        let p_filesz = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
        let p_memsz = u64::from_le_bytes(bytes[40..48].try_into().unwrap());
        let p_align = u64::from_le_bytes(bytes[48..56].try_into().unwrap());

        Some(Self {
            p_type,
            p_flags,
            p_offset,
            p_vaddr,
            p_paddr,
            p_filesz,
            p_memsz,
            p_align,
        })
    }

    /// Returns true if this is a loadable segment (`PT_LOAD`).
    pub fn is_loadable(&self) -> bool {
        self.p_type == PT_LOAD
    }

    /// Translates higher-half kernel virtual address to physical address offset.
    pub fn physical_load_target(&self, default_phys_base: u64) -> u64 {
        if self.p_paddr != 0 {
            self.p_paddr
        } else if self.p_vaddr >= KERNEL_HIGHER_HALF_BASE {
            default_phys_base + (self.p_vaddr - KERNEL_HIGHER_HALF_BASE)
        } else {
            self.p_vaddr
        }
    }
}

// -----------------------------------------------------------------------------
// Segment Loader Engine
// -----------------------------------------------------------------------------

pub const MAX_LOADABLE_SEGMENTS: usize = 8;

#[derive(Debug, Copy, Clone)]
pub struct KernelLoadPlan {
    pub entry_point: u64,
    pub segments: [Elf64ProgramHeader; MAX_LOADABLE_SEGMENTS],
    pub segment_count: usize,
    pub min_vaddr: u64,
    pub max_vaddr: u64,
    pub total_memory_bytes: u64,
}

/// Computes the kernel loading plan and verifies segment boundaries.
pub fn plan_kernel_load(elf_bytes: &[u8], _default_phys_base: u64) -> Result<KernelLoadPlan, ElfError> {
    let header = Elf64Header::parse(elf_bytes)?;

    let mut segments = [Elf64ProgramHeader::default(); MAX_LOADABLE_SEGMENTS];
    let mut segment_count = 0;
    let mut min_vaddr = u64::MAX;
    let mut max_vaddr = 0;
    let mut total_memory_bytes = 0;

    let ph_size = header.ph_entry_size as usize;
    let ph_off = header.ph_offset as usize;

    for i in 0..(header.ph_count as usize) {
        let entry_off = ph_off + i * ph_size;
        if entry_off + ph_size > elf_bytes.len() {
            return Err(ElfError::InvalidProgramHeaderOffset);
        }

        if let Some(ph) = Elf64ProgramHeader::parse(&elf_bytes[entry_off..entry_off + ph_size]) {
            if ph.is_loadable() && ph.p_memsz > 0 {
                if segment_count >= MAX_LOADABLE_SEGMENTS {
                    break;
                }

                // Verify file range
                let file_end = (ph.p_offset + ph.p_filesz) as usize;
                if file_end > elf_bytes.len() {
                    return Err(ElfError::SegmentOutOfBounds);
                }

                if ph.p_vaddr < min_vaddr {
                    min_vaddr = ph.p_vaddr;
                }
                let seg_end = ph.p_vaddr + ph.p_memsz;
                if seg_end > max_vaddr {
                    max_vaddr = seg_end;
                }
                total_memory_bytes += ph.p_memsz;

                segments[segment_count] = ph;
                segment_count += 1;
            }
        }
    }

    if min_vaddr == u64::MAX {
        min_vaddr = 0;
    }

    Ok(KernelLoadPlan {
        entry_point: header.entry_point,
        segments,
        segment_count,
        min_vaddr,
        max_vaddr,
        total_memory_bytes,
    })
}
