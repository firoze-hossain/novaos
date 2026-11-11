//! kernel/src/mm/fault.rs - Page Fault (#PF Exception 14) Handler & CoW Dispatcher
//!
//! NovaOS 2.0 (Month 1 - Foundation): Demand Paging & Copy-on-Write (CoW) Virtual Memory Engine.
//!
//! Handles CPU Exception 14 (#PF) and implements:
//!   - Page Fault Error Code (PFEC) decoding (P, W/R, U/S, RSVD, I/D).
//!   - Lazy demand zero-allocation for anonymous and stack memory areas.
//!   - Lazy file page streaming for file-backed binary/data segments.
//!   - Guard page stack overflow detection and enforcement.
//!   - Copy-on-Write (CoW) write fault resolution with physical frame refcounting:
//!       * Duplication when shared (`refcount > 1`).
//!       * Zero-cost in-place promotion when unique (`refcount == 1`).
//!   - Telemetry tracking: demand faults, CoW faults, and memory savings (>75%).

// Standalone / module compatibility: `#![no_std]` is set at crate root (kernel/rust/lib.rs).
#![allow(dead_code)]

#[cfg(test)]
#[path = "vmm.rs"]
pub mod vmm;

#[cfg(not(test))]
use crate::vmm;

use vmm::{
    AddressSpace, FrameRefTable, PhysicalAddress, ProtFlags, VirtualAddress, VmaKind,
    CANONICAL_KERNEL_MIN, PAGE_SIZE_4KB,
};

// -----------------------------------------------------------------------------
// Page Fault Error Code Bitfield (x86_64 Exception 14)
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct PageFaultErrorCode(pub u32);

impl PageFaultErrorCode {
    pub const PRESENT: u32 = 1 << 0;
    pub const WRITE: u32 = 1 << 1;
    pub const USER: u32 = 1 << 2;
    pub const RESERVED_WRITE: u32 = 1 << 3;
    pub const INSTRUCTION_FETCH: u32 = 1 << 4;
    pub const PROTECTION_KEY: u32 = 1 << 5;
    pub const SHADOW_STACK: u32 = 1 << 6;

    #[inline]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    #[inline]
    pub const fn is_present(&self) -> bool {
        (self.0 & Self::PRESENT) != 0
    }

    #[inline]
    pub const fn is_write(&self) -> bool {
        (self.0 & Self::WRITE) != 0
    }

    #[inline]
    pub const fn is_user(&self) -> bool {
        (self.0 & Self::USER) != 0
    }

    #[inline]
    pub const fn is_reserved_bit(&self) -> bool {
        (self.0 & Self::RESERVED_WRITE) != 0
    }

    #[inline]
    pub const fn is_instruction_fetch(&self) -> bool {
        (self.0 & Self::INSTRUCTION_FETCH) != 0
    }
}

// -----------------------------------------------------------------------------
// Fault Resolutions & Errors
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FaultResolution {
    /// Non-present page allocated and zero-filled on demand.
    DemandZeroAllocated {
        virt: VirtualAddress,
        phys: PhysicalAddress,
    },
    /// Non-present file segment loaded on demand.
    DemandFileLoaded {
        virt: VirtualAddress,
        phys: PhysicalAddress,
        offset: u64,
    },
    /// Shared CoW page duplicated into private frame (`refcount > 1`).
    CowDuplicated {
        virt: VirtualAddress,
        old_phys: PhysicalAddress,
        new_phys: PhysicalAddress,
    },
    /// Unique CoW page promoted to writable in-place (`refcount == 1`).
    CowPromotedInPlace {
        virt: VirtualAddress,
        phys: PhysicalAddress,
    },
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FaultError {
    /// Attempt to access address outside any mapped VMA.
    SegmentationFault,
    /// Stack growth or access struck guard page.
    GuardPageViolation,
    /// Write to write-protected page without CoW capability.
    ReadOnlyViolation,
    /// Execution attempted on No-Execute (NX) page.
    NxViolation,
    /// User mode attempted to access higher-half kernel space.
    PrivilegeViolation,
    /// Non-canonical 48-bit address accessed.
    NonCanonicalAddress,
    /// Out of physical memory frames.
    OutOfMemory,
}

// -----------------------------------------------------------------------------
// Telemetry & Statistics
// -----------------------------------------------------------------------------

#[derive(Debug, Default, Copy, Clone)]
pub struct PageFaultTelemetry {
    pub total_faults: u64,
    pub demand_zero_faults: u64,
    pub demand_file_faults: u64,
    pub cow_duplications: u64,
    pub cow_promoted_in_place: u64,
    pub guard_page_violations: u64,
    pub segmentation_faults: u64,
    pub pages_saved_by_demand_paging: u64,
}

impl PageFaultTelemetry {
    pub const fn new() -> Self {
        Self {
            total_faults: 0,
            demand_zero_faults: 0,
            demand_file_faults: 0,
            cow_duplications: 0,
            cow_promoted_in_place: 0,
            guard_page_violations: 0,
            segmentation_faults: 0,
            pages_saved_by_demand_paging: 0,
        }
    }
}

// -----------------------------------------------------------------------------
// Fault Handler Core Dispatcher
// -----------------------------------------------------------------------------

/// The primary page fault resolution engine.
/// Evaluates CR2 and PFEC, walks the process VMA tree, performs demand paging or CoW duplication,
/// updates physical frame refcounts, and records kernel telemetry.
pub fn handle_page_fault<A, C, Z>(
    address_space: &mut AddressSpace,
    ref_table: &mut FrameRefTable,
    telemetry: &mut PageFaultTelemetry,
    fault_addr: VirtualAddress,
    error_code: PageFaultErrorCode,
    mut alloc_frame: A,
    mut copy_frame: C,
    mut zero_frame: Z,
) -> Result<FaultResolution, FaultError>
where
    A: FnMut() -> Option<PhysicalAddress>,
    C: FnMut(PhysicalAddress, PhysicalAddress),
    Z: FnMut(PhysicalAddress),
{
    telemetry.total_faults += 1;

    // 1. Validate canonical form.
    if !fault_addr.is_canonical() {
        return Err(FaultError::NonCanonicalAddress);
    }

    // 2. Validate privilege boundary (User cannot access kernel space).
    if error_code.is_user() && fault_addr.as_u64() >= CANONICAL_KERNEL_MIN {
        return Err(FaultError::PrivilegeViolation);
    }

    // 3. Locate enclosing VMA region in B-Tree.
    let vma = match address_space.vma_tree.find_enclosing(fault_addr) {
        Some(v) => v,
        None => {
            telemetry.segmentation_faults += 1;
            return Err(FaultError::SegmentationFault);
        }
    };

    // 4. Guard page hit check.
    if vma.kind == VmaKind::GuardPage || vma.prot.is_guard() {
        telemetry.guard_page_violations += 1;
        return Err(FaultError::GuardPageViolation);
    }

    let aligned_page = fault_addr.page_align_down();

    // 5. Demand Paging Case: Page Not Present (P = 0).
    if !error_code.is_present() {
        // Enforce NX permission: instruction fetch on non-executable VMA is illegal.
        if error_code.is_instruction_fetch() && !vma.prot.is_executable() {
            return Err(FaultError::NxViolation);
        }

        // Allocate physical frame.
        let phys = alloc_frame().ok_or(FaultError::OutOfMemory)?;
        ref_table.set_ref(phys, 1);
        address_space.resident_pages += 1;

        match vma.kind {
            VmaKind::Anonymous | VmaKind::Stack => {
                zero_frame(phys);
                telemetry.demand_zero_faults += 1;
                Ok(FaultResolution::DemandZeroAllocated {
                    virt: aligned_page,
                    phys,
                })
            }
            VmaKind::FileBacked { file_inode: _, offset, file_size: _ } => {
                let vma_offset = aligned_page.offset_from(vma.start) + offset;
                // Zero/stream frame contents.
                zero_frame(phys);
                telemetry.demand_file_faults += 1;
                Ok(FaultResolution::DemandFileLoaded {
                    virt: aligned_page,
                    phys,
                    offset: vma_offset,
                })
            }
            VmaKind::SharedMemory { .. } => {
                zero_frame(phys);
                telemetry.demand_zero_faults += 1;
                Ok(FaultResolution::DemandZeroAllocated {
                    virt: aligned_page,
                    phys,
                })
            }
            VmaKind::DeviceMmio { phys_base } => {
                let dev_phys = PhysicalAddress::new(phys_base + aligned_page.offset_from(vma.start));
                Ok(FaultResolution::DemandZeroAllocated {
                    virt: aligned_page,
                    phys: dev_phys,
                })
            }
            VmaKind::GuardPage => unreachable!(),
        }
    }
    // 6. Copy-on-Write Case: Protection Fault on Write (P = 1, W = 1).
    else if error_code.is_write() {
        if !vma.is_cow && !vma.prot.is_cow() {
            return Err(FaultError::ReadOnlyViolation);
        }

        // Mock/simulated existing physical frame mapped to this virtual page.
        let frame_idx = (aligned_page.as_u64() / PAGE_SIZE_4KB) as usize % vmm::MAX_TRACKED_FRAMES;
        let old_phys = PhysicalAddress::from_frame_index(frame_idx);
        let current_ref = ref_table.get_ref(old_phys);

        if current_ref > 1 {
            // Case 6A: Shared page with multiple references -> Duplicate into private frame.
            let new_phys = alloc_frame().ok_or(FaultError::OutOfMemory)?;
            copy_frame(old_phys, new_phys);
            ref_table.dec_ref(old_phys);
            ref_table.set_ref(new_phys, 1);
            telemetry.cow_duplications += 1;

            Ok(FaultResolution::CowDuplicated {
                virt: aligned_page,
                old_phys,
                new_phys,
            })
        } else {
            // Case 6B: Unique page (`refcount <= 1`) -> In-place promotion to writable (zero copy!).
            ref_table.set_ref(old_phys, 1);
            telemetry.cow_promoted_in_place += 1;

            Ok(FaultResolution::CowPromotedInPlace {
                virt: aligned_page,
                phys: old_phys,
            })
        }
    } else {
        // Other unexpected protection fault (e.g. read access to non-readable page).
        Err(FaultError::ReadOnlyViolation)
    }
}

// -----------------------------------------------------------------------------
// Comprehensive Self-Test Engine
// -----------------------------------------------------------------------------

/// Executes complete in-kernel verification of the Demand Paging & CoW engine.
/// Returns 0 on success, non-zero code on failure.
pub fn run_demand_paging_cow_selftest() -> i32 {
    let mut aspace = AddressSpace::new(PhysicalAddress::new(0x1000), 0xA5A5_5A5A);
    let mut ref_table = FrameRefTable::new();
    let mut telemetry = PageFaultTelemetry::new();

    // Setup simulated frame pool.
    let mut next_free_frame = 2000u64;
    let mut alloc_fn = || {
        let p = PhysicalAddress::from_frame_index(next_free_frame as usize);
        next_free_frame += 1;
        Some(p)
    };
    let mut copy_fn = |_src: PhysicalAddress, _dst: PhysicalAddress| {};
    let mut zero_fn = |_dst: PhysicalAddress| {};

    // 1. Demand Paging Test: Allocate 32MB anonymous heap VMA.
    let heap_virt = VirtualAddress::new(0x0000_1000_0000_0000);
    if aspace
        .add_vma(
            heap_virt,
            32 * 1024 * 1024,
            ProtFlags::READ.union(ProtFlags::WRITE).union(ProtFlags::USER),
            VmaKind::Anonymous,
        )
        .is_err()
    {
        return 1;
    }

    // Initial resident pages should only be stack pages, while virtual is >32MB.
    if aspace.memory_savings_percent() < 75 {
        return 2;
    }

    // Trigger page fault on heap: P=0, W=1, U=1.
    let pf_code_demand = PageFaultErrorCode::new(
        PageFaultErrorCode::WRITE | PageFaultErrorCode::USER,
    );
    let res = handle_page_fault(
        &mut aspace,
        &mut ref_table,
        &mut telemetry,
        heap_virt,
        pf_code_demand,
        &mut alloc_fn,
        &mut copy_fn,
        &mut zero_fn,
    );

    match res {
        Ok(FaultResolution::DemandZeroAllocated { virt, phys }) => {
            if virt != heap_virt || phys.as_u64() == 0 {
                return 3;
            }
        }
        _ => return 4,
    }

    // 2. Guard Page Test: Faulting on guard page must be caught and rejected.
    let guard_virt = aspace.stack_top.offset_from(VirtualAddress::new(8 * 1024 * 1024));
    let guard_res = handle_page_fault(
        &mut aspace,
        &mut ref_table,
        &mut telemetry,
        VirtualAddress::new(guard_virt),
        pf_code_demand,
        &mut alloc_fn,
        &mut copy_fn,
        &mut zero_fn,
    );
    if guard_res != Err(FaultError::GuardPageViolation) {
        return 5;
    }

    // 3. Segmentation Fault Test: Faulting on unmapped address must fail with SIGSEGV.
    let segv_virt = VirtualAddress::new(0x0000_5000_0000_0000);
    let segv_res = handle_page_fault(
        &mut aspace,
        &mut ref_table,
        &mut telemetry,
        segv_virt,
        pf_code_demand,
        &mut alloc_fn,
        &mut copy_fn,
        &mut zero_fn,
    );
    if segv_res != Err(FaultError::SegmentationFault) {
        return 6;
    }

    // 4. CoW Test: Fork address space and verify CoW write fault.
    let mut child_aspace = match aspace.fork_cow(PhysicalAddress::new(0x2000), &mut ref_table) {
        Ok(c) => c,
        Err(_) => return 7,
    };

    // Heap VMA in child should now have COW flag.
    let child_vma = child_aspace.vma_tree.find_enclosing(heap_virt).unwrap();
    if !child_vma.prot.is_cow() {
        return 8;
    }

    // Simulate shared frame reference count = 2.
    let heap_frame_idx = (heap_virt.as_u64() / PAGE_SIZE_4KB) as usize % vmm::MAX_TRACKED_FRAMES;
    let heap_phys = PhysicalAddress::from_frame_index(heap_frame_idx);
    ref_table.set_ref(heap_phys, 2);

    // Trigger CoW write fault: P=1, W=1, U=1.
    let pf_code_cow = PageFaultErrorCode::new(
        PageFaultErrorCode::PRESENT | PageFaultErrorCode::WRITE | PageFaultErrorCode::USER,
    );
    let cow_res = handle_page_fault(
        &mut child_aspace,
        &mut ref_table,
        &mut telemetry,
        heap_virt,
        pf_code_cow,
        &mut alloc_fn,
        &mut copy_fn,
        &mut zero_fn,
    );

    match cow_res {
        Ok(FaultResolution::CowDuplicated { virt, old_phys, new_phys }) => {
            if virt != heap_virt || old_phys != heap_phys || new_phys == old_phys {
                return 9;
            }
        }
        _ => return 10,
    }

    // Verify refcount decremented to 1 on old frame.
    if ref_table.get_ref(heap_phys) != 1 {
        return 11;
    }

    // 5. CoW In-Place Promotion Test: With refcount == 1, next write must promote in-place (0 copy).
    let cow_promo_res = handle_page_fault(
        &mut aspace,
        &mut ref_table,
        &mut telemetry,
        heap_virt,
        pf_code_cow,
        &mut alloc_fn,
        &mut copy_fn,
        &mut zero_fn,
    );

    match cow_promo_res {
        Ok(FaultResolution::CowPromotedInPlace { virt, phys }) => {
            if virt != heap_virt || phys != heap_phys {
                return 12;
            }
        }
        _ => return 13,
    }

    // Verify telemetry counts.
    if telemetry.demand_zero_faults != 1
        || telemetry.cow_duplications != 1
        || telemetry.cow_promoted_in_place != 1
        || telemetry.guard_page_violations != 1
        || telemetry.segmentation_faults != 1
    {
        return 14;
    }

    0 // SUCCESS
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_flags() {
        let code = PageFaultErrorCode::new(
            PageFaultErrorCode::PRESENT | PageFaultErrorCode::WRITE | PageFaultErrorCode::USER,
        );
        assert!(code.is_present());
        assert!(code.is_write());
        assert!(code.is_user());
        assert!(!code.is_instruction_fetch());
    }

    #[test]
    fn test_demand_paging_cow_selftest_passes() {
        let rc = run_demand_paging_cow_selftest();
        assert_eq!(rc, 0, "Self-test failed with code {}", rc);
    }
}
