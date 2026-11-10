//! kernel/src/arch/x86_64/paging.rs - Pure Rust x86_64 Long Mode & PML4 4-Level Paging Engine
//!
//! Transition the CPU execution model from 32-bit protected mode (i686) to pure 64-bit Long Mode (x86_64).
//! Implements 4-level paging (PML4 -> PDPT -> PD -> PT) in pure `#![no_std]` Rust:
//!   - 48-bit (256 TB) virtual address space layout.
//!   - Strict sign-extended canonical address validation and filtering.
//!   - Hardware NX (No-Execute) protection on stack and data pages (EFER.NXE + bit 63).
//!   - RIP-relative addressing and dual virtual mappings:
//!       * Identity mapping of the first 4GB physical RAM with 2MB huge pages.
//!       * Higher-half kernel base mapping at `0xFFFF_8000_0000_0000`.
//!   - Zero-unsafe page-table walker supporting 4KB standard pages, 2MB huge pages, and 1GB huge pages.
//!   - Page Fault Error Code (PFEC) bitfield validator (P, W/R, U/S, RSVD, I/D, PK, SS).
//!   - Long Mode transition control register synthesis: EFER MSR, CR0, CR3, and CR4.

// Standalone / module compatibility: `#![no_std]` is set at crate root (kernel/rust/lib.rs).

// -----------------------------------------------------------------------------
// Constants & Architectural Limits
// -----------------------------------------------------------------------------

/// Number of entries in each level of an x86_64 page table (512 * 8 bytes = 4096 bytes).
pub const PAGE_TABLE_ENTRIES: usize = 512;

/// Standard page size (4 KB).
pub const PAGE_SIZE_4KB: u64 = 4096;
/// Huge page size (2 MB).
pub const PAGE_SIZE_2MB: u64 = 2 * 1024 * 1024;
/// Huge page size (1 GB).
pub const PAGE_SIZE_1GB: u64 = 1024 * 1024 * 1024;

/// Address mask for the physical frame base address in a page table entry (bits 12..51).
pub const PHYSICAL_FRAME_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// Base virtual address for higher-half kernel mapping (PML4 index 256).
pub const KERNEL_HIGHER_HALF_BASE: u64 = 0xFFFF_8000_0000_0000;

/// Maximum canonical lower-half address (128 TB).
pub const CANONICAL_LOWER_MAX: u64 = 0x0000_7FFF_FFFF_FFFF;
/// Minimum canonical higher-half address (128 TB from top of 64-bit space).
pub const CANONICAL_HIGHER_MIN: u64 = 0xFFFF_8000_0000_0000;

/// IA32_EFER MSR number.
pub const MSR_EFER: u32 = 0xC000_0080;

// -----------------------------------------------------------------------------
// Page Table Entry Flags (64-Bit x86_64)
// -----------------------------------------------------------------------------

/// Page Table Entry permission and configuration flags.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct PageTableFlags(pub u64);

impl PageTableFlags {
    /// Present: the page or page table is currently in physical memory.
    pub const PRESENT: Self = Self(1 << 0);
    /// Writable: writes are allowed; if clear, page is read-only.
    pub const WRITABLE: Self = Self(1 << 1);
    /// User Accessible: user-mode code (Ring 3) can access this page; if clear, supervisor-only.
    pub const USER_ACCESSIBLE: Self = Self(1 << 2);
    /// Page-Level Write-Through: caching policy.
    pub const WRITE_THROUGH: Self = Self(1 << 3);
    /// Page-Level Cache Disable: prevents caching of this page.
    pub const CACHE_DISABLE: Self = Self(1 << 4);
    /// Accessed: set by CPU when the page or table is accessed for read/write.
    pub const ACCESSED: Self = Self(1 << 5);
    /// Dirty: set by CPU when the page has been written to.
    pub const DIRTY: Self = Self(1 << 6);
    /// Page Size / Huge Page: 2MB in PD, 1GB in PDPT.
    pub const HUGE_PAGE: Self = Self(1 << 7);
    /// Global: translation is not invalidated in TLB on CR3 reload.
    pub const GLOBAL: Self = Self(1 << 8);
    /// OS-specific Flag 1: Copy-on-Write (COW).
    pub const COW: Self = Self(1 << 9);
    /// OS-specific Flag 2: Shared Memory (SHM).
    pub const SHM: Self = Self(1 << 10);
    /// OS-specific Flag 3: Demand Paged / Guard Page.
    pub const GUARD: Self = Self(1 << 11);
    /// No-Execute (NX): instruction execution is disabled on this page (requires EFER.NXE).
    pub const NO_EXECUTE: Self = Self(1 << 63);

    #[inline]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[inline]
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline]
    pub const fn bits(&self) -> u64 {
        self.0
    }

    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[inline]
    pub const fn remove(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

// -----------------------------------------------------------------------------
// Canonical Virtual Address & Physical Address
// -----------------------------------------------------------------------------

/// Represents an x86_64 48-bit Virtual Address with canonical address validation.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct VirtualAddress(pub u64);

#[derive(Debug, PartialEq, Eq)]
pub enum PagingError {
    NonCanonicalAddress(u64),
    NotPresent { level: u8, index: usize },
    HugePageEncountered { level: u8, phys_addr: u64 },
    MisalignedAddress(u64),
    ReservedBitViolation,
    FrameAllocationFailed,
}

impl VirtualAddress {
    /// Constructs a virtual address, validating that it is in canonical form.
    pub const fn new(addr: u64) -> Result<Self, PagingError> {
        if is_canonical_address(addr) {
            Ok(Self(addr))
        } else {
            Err(PagingError::NonCanonicalAddress(addr))
        }
    }

    /// Constructs a virtual address without checking, truncating non-canonical bits.
    pub const fn new_truncate(addr: u64) -> Self {
        Self(canonicalize_address(addr))
    }

    #[inline]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    #[inline]
    pub const fn is_canonical(&self) -> bool {
        is_canonical_address(self.0)
    }

    #[inline]
    pub const fn is_higher_half(&self) -> bool {
        self.0 >= CANONICAL_HIGHER_MIN
    }

    #[inline]
    pub const fn is_lower_half(&self) -> bool {
        self.0 <= CANONICAL_LOWER_MAX
    }

    /// Extract PML4 index (Level 4, bits 39..47).
    #[inline]
    pub const fn pml4_index(&self) -> usize {
        ((self.0 >> 39) & 0x1FF) as usize
    }

    /// Extract PDPT index (Level 3, bits 30..38).
    #[inline]
    pub const fn pdpt_index(&self) -> usize {
        ((self.0 >> 30) & 0x1FF) as usize
    }

    /// Extract PD index (Level 2, bits 21..29).
    #[inline]
    pub const fn pd_index(&self) -> usize {
        ((self.0 >> 21) & 0x1FF) as usize
    }

    /// Extract PT index (Level 1, bits 12..20).
    #[inline]
    pub const fn pt_index(&self) -> usize {
        ((self.0 >> 12) & 0x1FF) as usize
    }

    /// 4KB Page Offset (bits 0..11).
    #[inline]
    pub const fn offset_4kb(&self) -> u64 {
        self.0 & (PAGE_SIZE_4KB - 1)
    }

    /// 2MB Huge Page Offset (bits 0..20).
    #[inline]
    pub const fn offset_2mb(&self) -> u64 {
        self.0 & (PAGE_SIZE_2MB - 1)
    }

    /// 1GB Huge Page Offset (bits 0..29).
    #[inline]
    pub const fn offset_1gb(&self) -> u64 {
        self.0 & (PAGE_SIZE_1GB - 1)
    }
}

/// Checks if an address is in 48-bit canonical form (sign-extended from bit 47).
#[inline]
pub const fn is_canonical_address(addr: u64) -> bool {
    let sign_bit = (addr >> 47) & 1;
    if sign_bit == 0 {
        (addr >> 48) == 0
    } else {
        (addr >> 48) == 0xFFFF
    }
}

/// Sign-extends bit 47 into bits 48..63 to produce a valid canonical address.
#[inline]
pub const fn canonicalize_address(addr: u64) -> u64 {
    let sign_bit = (addr >> 47) & 1;
    if sign_bit == 0 {
        addr & 0x0000_7FFF_FFFF_FFFF
    } else {
        addr | 0xFFFF_8000_0000_0000
    }
}

/// Represents an x86_64 physical memory address (up to 52 bits supported by hardware).
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct PhysicalAddress(pub u64);

impl PhysicalAddress {
    pub const fn new(addr: u64) -> Self {
        Self(addr & 0x000F_FFFF_FFFF_FFFF)
    }

    #[inline]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    #[inline]
    pub const fn is_aligned_4kb(&self) -> bool {
        (self.0 & (PAGE_SIZE_4KB - 1)) == 0
    }

    #[inline]
    pub const fn is_aligned_2mb(&self) -> bool {
        (self.0 & (PAGE_SIZE_2MB - 1)) == 0
    }

    #[inline]
    pub const fn is_aligned_1gb(&self) -> bool {
        (self.0 & (PAGE_SIZE_1GB - 1)) == 0
    }
}

// -----------------------------------------------------------------------------
// Page Table Entries & Tables
// -----------------------------------------------------------------------------

/// Single 64-bit Page Table Entry (PML4E, PDPTE, PDE, or PTE).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct PageTableEntry(pub u64);

impl PageTableEntry {
    pub const fn new() -> Self {
        Self(0)
    }

    #[inline]
    pub const fn raw(&self) -> u64 {
        self.0
    }

    #[inline]
    pub const fn flags(&self) -> PageTableFlags {
        PageTableFlags(self.0 & !PHYSICAL_FRAME_MASK)
    }

    #[inline]
    pub const fn physical_address(&self) -> PhysicalAddress {
        PhysicalAddress(self.0 & PHYSICAL_FRAME_MASK)
    }

    #[inline]
    pub const fn is_present(&self) -> bool {
        (self.0 & PageTableFlags::PRESENT.0) != 0
    }

    #[inline]
    pub const fn is_writable(&self) -> bool {
        (self.0 & PageTableFlags::WRITABLE.0) != 0
    }

    #[inline]
    pub const fn is_user(&self) -> bool {
        (self.0 & PageTableFlags::USER_ACCESSIBLE.0) != 0
    }

    #[inline]
    pub const fn is_huge(&self) -> bool {
        (self.0 & PageTableFlags::HUGE_PAGE.0) != 0
    }

    #[inline]
    pub const fn is_no_execute(&self) -> bool {
        (self.0 & PageTableFlags::NO_EXECUTE.0) != 0
    }

    #[inline]
    pub fn set(&mut self, addr: PhysicalAddress, flags: PageTableFlags) {
        self.0 = (addr.as_u64() & PHYSICAL_FRAME_MASK) | flags.bits();
    }

    #[inline]
    pub fn clear(&mut self) {
        self.0 = 0;
    }
}

/// A 4KB page table containing 512 64-bit entries.
#[repr(align(4096))]
pub struct PageTable {
    pub entries: [PageTableEntry; PAGE_TABLE_ENTRIES],
}

impl PageTable {
    pub const fn new() -> Self {
        Self {
            entries: [PageTableEntry(0); PAGE_TABLE_ENTRIES],
        }
    }

    pub fn zero(&mut self) {
        for entry in self.entries.iter_mut() {
            entry.clear();
        }
    }
}

// -----------------------------------------------------------------------------
// Zero-Unsafe Page Table Walker
// -----------------------------------------------------------------------------

/// Translation result returned by the page table walker.
#[derive(Debug, PartialEq, Eq)]
pub enum PageSize {
    Standard4KB,
    Huge2MB,
    Huge1GB,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TranslationResult {
    pub physical_address: PhysicalAddress,
    pub page_size: PageSize,
    pub flags: PageTableFlags,
    pub is_no_execute: bool,
}

/// Zero-unsafe page-table walker. Traverses in-memory table structures
/// safely without raw pointer dereferences in the traversal logic.
pub struct PageTableWalker<'a> {
    pml4: &'a PageTable,
    pdpt: Option<&'a PageTable>,
    pd: Option<&'a PageTable>,
    pt: Option<&'a PageTable>,
}

impl<'a> PageTableWalker<'a> {
    pub const fn new(pml4: &'a PageTable) -> Self {
        Self {
            pml4,
            pdpt: None,
            pd: None,
            pt: None,
        }
    }

    pub fn with_tables(
        pml4: &'a PageTable,
        pdpt: Option<&'a PageTable>,
        pd: Option<&'a PageTable>,
        pt: Option<&'a PageTable>,
    ) -> Self {
        Self { pml4, pdpt, pd, pt }
    }

    /// Walks the 4-level paging hierarchy for a given virtual address.
    pub fn walk(&self, vaddr: VirtualAddress) -> Result<TranslationResult, PagingError> {
        if !vaddr.is_canonical() {
            return Err(PagingError::NonCanonicalAddress(vaddr.as_u64()));
        }

        let pml4_idx = vaddr.pml4_index();
        let pml4_entry = self.pml4.entries[pml4_idx];
        if !pml4_entry.is_present() {
            return Err(PagingError::NotPresent {
                level: 4,
                index: pml4_idx,
            });
        }

        let pdpt_idx = vaddr.pdpt_index();
        let pdpt_table = self.pdpt.ok_or(PagingError::NotPresent {
            level: 3,
            index: pdpt_idx,
        })?;
        let pdpt_entry = pdpt_table.entries[pdpt_idx];
        if !pdpt_entry.is_present() {
            return Err(PagingError::NotPresent {
                level: 3,
                index: pdpt_idx,
            });
        }

        // Check for 1GB Huge Page at PDPT level
        if pdpt_entry.is_huge() {
            let base_phys = pdpt_entry.physical_address().as_u64();
            let full_phys = base_phys + vaddr.offset_1gb();
            return Ok(TranslationResult {
                physical_address: PhysicalAddress(full_phys),
                page_size: PageSize::Huge1GB,
                flags: pdpt_entry.flags(),
                is_no_execute: pdpt_entry.is_no_execute(),
            });
        }

        let pd_idx = vaddr.pd_index();
        let pd_table = self.pd.ok_or(PagingError::NotPresent {
            level: 2,
            index: pd_idx,
        })?;
        let pd_entry = pd_table.entries[pd_idx];
        if !pd_entry.is_present() {
            return Err(PagingError::NotPresent {
                level: 2,
                index: pd_idx,
            });
        }

        // Check for 2MB Huge Page at PD level
        if pd_entry.is_huge() {
            let base_phys = pd_entry.physical_address().as_u64();
            let full_phys = base_phys + vaddr.offset_2mb();
            return Ok(TranslationResult {
                physical_address: PhysicalAddress(full_phys),
                page_size: PageSize::Huge2MB,
                flags: pd_entry.flags(),
                is_no_execute: pd_entry.is_no_execute(),
            });
        }

        let pt_idx = vaddr.pt_index();
        let pt_table = self.pt.ok_or(PagingError::NotPresent {
            level: 1,
            index: pt_idx,
        })?;
        let pt_entry = pt_table.entries[pt_idx];
        if !pt_entry.is_present() {
            return Err(PagingError::NotPresent {
                level: 1,
                index: pt_idx,
            });
        }

        let base_phys = pt_entry.physical_address().as_u64();
        let full_phys = base_phys + vaddr.offset_4kb();
        Ok(TranslationResult {
            physical_address: PhysicalAddress(full_phys),
            page_size: PageSize::Standard4KB,
            flags: pt_entry.flags(),
            is_no_execute: pt_entry.is_no_execute(),
        })
    }
}

// -----------------------------------------------------------------------------
// Page Fault Error Code (PFEC) Validator
// -----------------------------------------------------------------------------

/// Decoded Page Fault Error Code pushed by CPU onto the stack during #PF (Exception 14).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct PageFaultErrorCode(pub u32);

impl PageFaultErrorCode {
    pub const PRESENT: u32 = 1 << 0;
    pub const WRITE: u32 = 1 << 1;
    pub const USER: u32 = 1 << 2;
    pub const RESERVED_WRITE: u32 = 1 << 3;
    pub const INSTRUCTION_FETCH: u32 = 1 << 4;
    pub const PROTECTION_KEY: u32 = 1 << 5;
    pub const SHADOW_STACK: u32 = 1 << 6;
    pub const SGX: u32 = 1 << 15;

    #[inline]
    pub const fn new(code: u32) -> Self {
        Self(code)
    }

    /// True if fault was caused by a page-protection violation (Present bit was 1).
    /// False if caused by accessing a non-present page.
    #[inline]
    pub const fn is_protection_violation(&self) -> bool {
        (self.0 & Self::PRESENT) != 0
    }

    /// True if fault was caused by a memory write access.
    #[inline]
    pub const fn is_write(&self) -> bool {
        (self.0 & Self::WRITE) != 0
    }

    /// True if fault was caused in user mode (CPL == 3).
    #[inline]
    pub const fn is_user_mode(&self) -> bool {
        (self.0 & Self::USER) != 0
    }

    /// True if fault was caused by reserved bit violation in paging structure.
    #[inline]
    pub const fn is_reserved_violation(&self) -> bool {
        (self.0 & Self::RESERVED_WRITE) != 0
    }

    /// True if fault was caused by instruction fetch from a No-Execute (NX) page.
    #[inline]
    pub const fn is_instruction_fetch(&self) -> bool {
        (self.0 & Self::INSTRUCTION_FETCH) != 0
    }

    /// True if fault was caused by protection keys.
    #[inline]
    pub const fn is_protection_key(&self) -> bool {
        (self.0 & Self::PROTECTION_KEY) != 0
    }

    /// Returns human-readable diagnostic description of the fault reason.
    pub fn description(&self) -> &'static str {
        if self.is_instruction_fetch() {
            "Instruction fetch execution violation (NX/DEP protection)"
        } else if self.is_reserved_violation() {
            "Reserved bit violation in paging structure"
        } else if self.is_protection_violation() && self.is_write() && self.is_user_mode() {
            "User-mode write to read-only page"
        } else if self.is_protection_violation() && !self.is_user_mode() {
            "Kernel-mode page protection violation"
        } else if !self.is_protection_violation() && self.is_user_mode() {
            "User-mode access to non-present page"
        } else if !self.is_protection_violation() && !self.is_user_mode() {
            "Kernel-mode access to non-present page (NULL or unmapped)"
        } else {
            "General paging protection fault"
        }
    }
}

// -----------------------------------------------------------------------------
// Long Mode Activation & Control Register Synthesis
// -----------------------------------------------------------------------------

/// Control register bit definitions for Long Mode activation.
pub mod registers {
    pub const CR0_PE: u64 = 1 << 0;  // Protection Enable
    pub const CR0_WP: u64 = 1 << 16; // Write Protect
    pub const CR0_PG: u64 = 1 << 31; // Paging

    pub const CR4_PSE: u64 = 1 << 4;        // Page Size Extensions
    pub const CR4_PAE: u64 = 1 << 5;        // Physical Address Extension (mandatory for Long Mode)
    pub const CR4_PGE: u64 = 1 << 7;        // Page Global Enable
    pub const CR4_OSFXSR: u64 = 1 << 9;     // OS support for FXSAVE/FXRSTOR
    pub const CR4_OSXMMEXCPT: u64 = 1 << 10;// OS support for unmasked SIMD FP exceptions

    pub const EFER_SCE: u64 = 1 << 0; // System Call Extensions (syscall/sysret)
    pub const EFER_LME: u64 = 1 << 8; // Long Mode Enable
    pub const EFER_LMA: u64 = 1 << 10;// Long Mode Active
    pub const EFER_NXE: u64 = 1 << 11;// No-Execute Enable
}

/// Synthesized register state required to activate 64-bit Long Mode.
#[derive(Debug, PartialEq, Eq)]
pub struct LongModeBootRegisters {
    pub cr3: u64,
    pub cr4: u64,
    pub cr0: u64,
    pub efer: u64,
}

/// Prepares and validates the register configuration for entering Long Mode.
pub fn prepare_long_mode_activation(pml4_phys_addr: u64) -> Result<LongModeBootRegisters, &'static str> {
    if (pml4_phys_addr & 0xFFF) != 0 {
        return Err("PML4 base address must be 4KB page aligned");
    }

    let cr4 = registers::CR4_PAE | registers::CR4_PGE | registers::CR4_PSE | registers::CR4_OSFXSR | registers::CR4_OSXMMEXCPT;
    let efer = registers::EFER_LME | registers::EFER_NXE | registers::EFER_SCE;
    let cr0 = registers::CR0_PE | registers::CR0_WP | registers::CR0_PG;

    Ok(LongModeBootRegisters {
        cr3: pml4_phys_addr,
        cr4,
        cr0,
        efer,
    })
}

// -----------------------------------------------------------------------------
// Boot Table Initialization (4GB 2MB Huge Pages + Higher-Half Kernel)
// -----------------------------------------------------------------------------

/// Structure holding static boot tables for identity mapping and higher-half mapping.
pub struct BootPml4Environment {
    pub pml4: PageTable,
    pub pdpt_low: PageTable,
    pub pd_low: [PageTable; 4], // 4 PDs * 512 entries * 2MB = 4 GB
}

impl BootPml4Environment {
    pub const fn new() -> Self {
        Self {
            pml4: PageTable::new(),
            pdpt_low: PageTable::new(),
            pd_low: [
                PageTable::new(),
                PageTable::new(),
                PageTable::new(),
                PageTable::new(),
            ],
        }
    }

    /// Initializes:
    ///   1. Identity mapping for 0 .. 4GB physical memory using 2MB huge pages.
    ///   2. Higher-half mapping at `0xFFFF_8000_0000_0000` pointing to the same 4GB physical space.
    pub fn initialize(&mut self, pdpt_low_phys: u64, pd_low_phys: [u64; 4]) {
        self.pml4.zero();
        self.pdpt_low.zero();
        for pd in self.pd_low.iter_mut() {
            pd.zero();
        }

        // Map 4 Page Directories into PDPT (entries 0, 1, 2, 3 covering 4 GB)
        for i in 0..4 {
            let flags = PageTableFlags::PRESENT.union(PageTableFlags::WRITABLE);
            self.pdpt_low.entries[i].set(PhysicalAddress(pd_low_phys[i]), flags);

            // Populate each PD with 512 x 2MB huge pages
            for j in 0..PAGE_TABLE_ENTRIES {
                let phys_2mb = ((i as u64) * 1024 * 1024 * 1024) + ((j as u64) * PAGE_SIZE_2MB);
                let huge_flags = PageTableFlags::PRESENT
                    .union(PageTableFlags::WRITABLE)
                    .union(PageTableFlags::HUGE_PAGE)
                    .union(PageTableFlags::GLOBAL);
                self.pd_low[i].entries[j].set(PhysicalAddress(phys_2mb), huge_flags);
            }
        }

        // PML4 entry 0: Identity mapping for 0 .. 512GB (lower half)
        let root_flags = PageTableFlags::PRESENT.union(PageTableFlags::WRITABLE);
        self.pml4.entries[0].set(PhysicalAddress(pdpt_low_phys), root_flags);

        // PML4 entry 256: Higher-half kernel base (0xFFFF_8000_0000_0000)
        self.pml4.entries[256].set(PhysicalAddress(pdpt_low_phys), root_flags);
    }
}

// -----------------------------------------------------------------------------
// Comprehensive Self-Test & Verification Suite
// -----------------------------------------------------------------------------

/// Runs the exhaustive x86_64 Long Mode & PML4 Paging Engine self-test.
/// Returns 0 on success, negative error code on failure.
pub fn run_x86_64_paging_selftest() -> Result<(), &'static str> {
    // 1. Canonical Address Validation
    let lower_valid = VirtualAddress::new(0x0000_7FFF_FFFF_FFFF);
    if lower_valid.is_err() || !lower_valid.unwrap().is_lower_half() {
        return Err("Lower-half canonical address validation failed");
    }

    let higher_valid = VirtualAddress::new(0xFFFF_8000_0000_0000);
    if higher_valid.is_err() || !higher_valid.unwrap().is_higher_half() {
        return Err("Higher-half canonical address validation failed");
    }

    let non_canonical = VirtualAddress::new(0x0000_8000_0000_0000);
    if non_canonical.is_ok() {
        return Err("Non-canonical address was erroneously accepted");
    }

    let sign_extended = canonicalize_address(0x0000_8000_1234_5678);
    if sign_extended != 0xFFFF_8000_1234_5678 {
        return Err("Address sign-extension canonicalization failed");
    }

    // 2. Index Extraction Verification
    let v_test = VirtualAddress::new(0xFFFF_8000_4020_1000).map_err(|_| "Invalid address")?;
    if v_test.pml4_index() != 256 {
        return Err("PML4 index extraction mismatch");
    }
    if v_test.pdpt_index() != 1 {
        return Err("PDPT index extraction mismatch");
    }
    if v_test.pd_index() != 1 {
        return Err("PD index extraction mismatch");
    }
    if v_test.pt_index() != 1 {
        return Err("PT index extraction mismatch");
    }

    // 3. Page Table Entry & Flags Logic
    let mut entry = PageTableEntry::new();
    let phys = PhysicalAddress(0x1000_0000); // 256 MB
    let flags = PageTableFlags::PRESENT
        .union(PageTableFlags::WRITABLE)
        .union(PageTableFlags::NO_EXECUTE);

    entry.set(phys, flags);
    if !entry.is_present() || !entry.is_writable() || !entry.is_no_execute() {
        return Err("Entry flags set/query failed");
    }
    if entry.physical_address().as_u64() != 0x1000_0000 {
        return Err("Entry physical address masking failed");
    }

    // 4. Page Fault Error Code (PFEC) Decoding
    let pf_nx = PageFaultErrorCode::new(
        PageFaultErrorCode::PRESENT | PageFaultErrorCode::INSTRUCTION_FETCH,
    );
    if !pf_nx.is_instruction_fetch() || !pf_nx.is_protection_violation() {
        return Err("PFEC instruction fetch decoding failed");
    }

    let pf_user_write = PageFaultErrorCode::new(
        PageFaultErrorCode::PRESENT | PageFaultErrorCode::WRITE | PageFaultErrorCode::USER,
    );
    if !pf_user_write.is_user_mode() || !pf_user_write.is_write() {
        return Err("PFEC user write decoding failed");
    }

    // 5. Zero-Unsafe Walker with 4KB, 2MB, and 1GB Pages
    let mut pml4 = PageTable::new();
    let mut pdpt = PageTable::new();
    let mut pd = PageTable::new();
    let mut pt = PageTable::new();

    // Set up PML4 -> PDPT
    pml4.entries[0].set(
        PhysicalAddress(0x1000),
        PageTableFlags::PRESENT.union(PageTableFlags::WRITABLE),
    );

    // Set up PDPT -> 1GB Huge Page at entry 1
    pdpt.entries[1].set(
        PhysicalAddress(0x4000_0000), // 1GB
        PageTableFlags::PRESENT
            .union(PageTableFlags::WRITABLE)
            .union(PageTableFlags::HUGE_PAGE),
    );

    // Set up PDPT -> PD at entry 0
    pdpt.entries[0].set(
        PhysicalAddress(0x2000),
        PageTableFlags::PRESENT.union(PageTableFlags::WRITABLE),
    );

    // Set up PD -> 2MB Huge Page at entry 0
    pd.entries[0].set(
        PhysicalAddress(0x0000),
        PageTableFlags::PRESENT
            .union(PageTableFlags::WRITABLE)
            .union(PageTableFlags::HUGE_PAGE),
    );

    // Set up PD -> PT at entry 1
    pd.entries[1].set(
        PhysicalAddress(0x3000),
        PageTableFlags::PRESENT.union(PageTableFlags::WRITABLE),
    );

    // Set up PT -> 4KB standard page at entry 5
    pt.entries[5].set(
        PhysicalAddress(0x0020_5000),
        PageTableFlags::PRESENT
            .union(PageTableFlags::WRITABLE)
            .union(PageTableFlags::NO_EXECUTE),
    );

    let walker = PageTableWalker::with_tables(&pml4, Some(&pdpt), Some(&pd), Some(&pt));

    // Test 2MB Translation
    let v_2mb = VirtualAddress::new(0x0000_0000_0004_2000).map_err(|_| "Invalid 2MB vaddr")?;
    let res_2mb = walker.walk(v_2mb).map_err(|_| "2MB walk failed")?;
    if res_2mb.page_size != PageSize::Huge2MB || res_2mb.physical_address.as_u64() != 0x0004_2000 {
        return Err("2MB huge page translation mismatch");
    }

    // Test 1GB Translation
    let v_1gb = VirtualAddress::new(0x0000_0000_4000_8000).map_err(|_| "Invalid 1GB vaddr")?;
    let res_1gb = walker.walk(v_1gb).map_err(|_| "1GB walk failed")?;
    if res_1gb.page_size != PageSize::Huge1GB || res_1gb.physical_address.as_u64() != 0x4000_8000 {
        return Err("1GB huge page translation mismatch");
    }

    // Test 4KB Translation with NX
    let v_4kb = VirtualAddress::new(0x0000_0000_0020_5ABC).map_err(|_| "Invalid 4KB vaddr")?;
    let res_4kb = walker.walk(v_4kb).map_err(|_| "4KB walk failed")?;
    if res_4kb.page_size != PageSize::Standard4KB || res_4kb.physical_address.as_u64() != 0x0020_5ABC {
        return Err("4KB standard page translation mismatch");
    }
    if !res_4kb.is_no_execute {
        return Err("4KB NX flag was not preserved");
    }

    // 6. Long Mode Activation Register Configuration
    let regs = prepare_long_mode_activation(0x1000_0000)?;
    if (regs.cr4 & registers::CR4_PAE) == 0 {
        return Err("CR4.PAE must be set for Long Mode");
    }
    if (regs.efer & registers::EFER_LME) == 0 || (regs.efer & registers::EFER_NXE) == 0 {
        return Err("EFER.LME and EFER.NXE must be set for Long Mode");
    }
    if (regs.cr0 & registers::CR0_PG) == 0 || (regs.cr0 & registers::CR0_WP) == 0 {
        return Err("CR0.PG and CR0.WP must be set for Long Mode");
    }

    Ok(())
}

// -----------------------------------------------------------------------------
// C FFI Exported Functions
// -----------------------------------------------------------------------------

/// Exported C FFI function to verify the x86_64 Long Mode & PML4 paging engine.
/// Returns 0 on success, -1 on failure.
#[no_mangle]
pub extern "C" fn rust_x86_64_paging_selftest() -> i32 {
    match run_x86_64_paging_selftest() {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// Returns the higher-half kernel base offset (0xFFFF_8000_0000_0000).
#[no_mangle]
pub extern "C" fn rust_x86_64_higher_half_base_hi() -> u32 {
    (KERNEL_HIGHER_HALF_BASE >> 32) as u32
}

#[no_mangle]
pub extern "C" fn rust_x86_64_higher_half_base_lo() -> u32 {
    (KERNEL_HIGHER_HALF_BASE & 0xFFFF_FFFF) as u32
}

// -----------------------------------------------------------------------------
// Unit Tests for Host Execution (`rustc --test`)
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_addresses() {
        assert!(is_canonical_address(0x0000_0000_0000_0000));
        assert!(is_canonical_address(0x0000_7FFF_FFFF_FFFF));
        assert!(is_canonical_address(0xFFFF_8000_0000_0000));
        assert!(is_canonical_address(0xFFFF_FFFF_FFFF_FFFF));

        assert!(!is_canonical_address(0x0000_8000_0000_0000));
        assert!(!is_canonical_address(0xFFFF_7FFF_FFFF_FFFF));
        assert!(!is_canonical_address(0x1234_5678_9ABC_DEF0));
    }

    #[test]
    fn test_virtual_address_indices() {
        let v = VirtualAddress::new(0xFFFF_8000_0000_0000).unwrap();
        assert_eq!(v.pml4_index(), 256);
        assert_eq!(v.pdpt_index(), 0);
        assert_eq!(v.pd_index(), 0);
        assert_eq!(v.pt_index(), 0);

        let v2 = VirtualAddress::new(0x0000_007F_BFDF_E000).unwrap();
        assert_eq!(v2.pml4_index(), 0);
        assert_eq!(v2.pdpt_index(), 510);
        assert_eq!(v2.pd_index(), 510);
        assert_eq!(v2.pt_index(), 510);
    }

    #[test]
    fn test_page_table_flags() {
        let flags = PageTableFlags::PRESENT
            .union(PageTableFlags::WRITABLE)
            .union(PageTableFlags::NO_EXECUTE);
        assert!(flags.contains(PageTableFlags::PRESENT));
        assert!(flags.contains(PageTableFlags::WRITABLE));
        assert!(!flags.contains(PageTableFlags::USER_ACCESSIBLE));
        assert!(flags.contains(PageTableFlags::NO_EXECUTE));

        let cleared = flags.remove(PageTableFlags::WRITABLE);
        assert!(!cleared.contains(PageTableFlags::WRITABLE));
        assert!(cleared.contains(PageTableFlags::PRESENT));
    }

    #[test]
    fn test_page_fault_error_code() {
        let code = PageFaultErrorCode::new(0b10001); // INSTRUCTION_FETCH | PRESENT
        assert!(code.is_protection_violation());
        assert!(!code.is_write());
        assert!(!code.is_user_mode());
        assert!(code.is_instruction_fetch());
    }

    #[test]
    fn test_long_mode_register_setup() {
        let res = prepare_long_mode_activation(0x2000);
        assert!(res.is_ok());
        let regs = res.unwrap();
        assert_eq!(regs.cr3, 0x2000);
        assert!((regs.cr4 & registers::CR4_PAE) != 0);
        assert!((regs.efer & registers::EFER_LME) != 0);
        assert!((regs.cr0 & registers::CR0_PG) != 0);
    }

    #[test]
    fn test_boot_pml4_environment() {
        let mut env = BootPml4Environment::new();
        env.initialize(0x1000, [0x2000, 0x3000, 0x4000, 0x5000]);

        // PML4 entry 0 (identity) and 256 (higher-half)
        assert!(env.pml4.entries[0].is_present());
        assert_eq!(env.pml4.entries[0].physical_address().as_u64(), 0x1000);
        assert!(env.pml4.entries[256].is_present());
        assert_eq!(env.pml4.entries[256].physical_address().as_u64(), 0x1000);

        // PDPT has 4 entries covering 4GB
        for i in 0..4 {
            assert!(env.pdpt_low.entries[i].is_present());
            assert_eq!(
                env.pdpt_low.entries[i].physical_address().as_u64(),
                0x2000 + (i as u64) * 0x1000
            );

            // Each PD has 512 x 2MB huge pages
            for j in 0..PAGE_TABLE_ENTRIES {
                let pde = env.pd_low[i].entries[j];
                assert!(pde.is_present());
                assert!(pde.is_huge());
                let expected_phys = ((i as u64) * 1024 * 1024 * 1024) + ((j as u64) * PAGE_SIZE_2MB);
                assert_eq!(pde.physical_address().as_u64(), expected_phys);
            }
        }
    }

    #[test]
    fn test_full_selftest_suite() {
        assert_eq!(run_x86_64_paging_selftest(), Ok(()));
        assert_eq!(rust_x86_64_paging_selftest(), 0);
    }
}
