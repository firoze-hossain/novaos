//! kernel/src/drivers/gpu/dmabuf.rs
//!
//! NovaOS 2.0 (Month 4 - Graphics & DRM): Zero-Copy DMA-BUF Framebuffer Sharing & GPU Memory Allocator
//!
//! Eliminates CPU pixel copying between userland applications, hardware accelerators,
//! and the display compositor (`novadwm`) by allocating hardware-backed GPU/DMA physical
//! memory pools and mapping the exact same physical 4KB frames simultaneously into client
//! and compositor virtual address spaces.
//!
//! Key Capabilities:
//! - Physical GPU/DMA Frame Pool Allocator (`GpuMemoryAllocator`): 512-word (128 MiB) bit-packed
//!   buddy/extent allocator supporting contiguous scanout buffers and scatter-gather (`SgTable`)
//!   non-contiguous fallback allocations.
//! - Scatter-Gather Table (`SgTable`): Tracks up to `MAX_SG_SEGMENTS` contiguous physical extents
//!   with O(segments) page translation for arbitrary virtual-to-physical lookups.
//! - Export / Import Lifecycle: Capability-controlled FD/PRIME handle passing (`export`, `import`),
//!   allowing a client to pass ownership/access rights of a GPU buffer to the compositor without
//!   copying a single pixel.
//! - Simultaneous Multi-Process MMU Mapping: Maps DMA-BUF physical frames into both client and
//!   compositor address spaces with unmapped trailing guard pages (`4096` bytes) to catch
//!   out-of-bounds GPU or UI draws.
//! - Cache Coherency & Synchronization (`sync`): Implements `DMA_BUF_SYNC_START` / `DMA_BUF_SYNC_END`
//!   with read/write memory barriers between CPU writers and GPU/Display readers.
//! - Reference-Counted Teardown & Process Crash Safety (`cleanup_process`): Ensures buffers remain
//!   valid while any process, FD, or scanout hardware attachment holds a reference, reclaiming
//!   100% of physical frames only when `refcount == 0`.

#![allow(dead_code, non_camel_case_types)]

// ============================================================================
// Constants & POSIX Errno Values (Positive inside module, negated at ABI edge)
// ============================================================================

pub const DMABUF_PAGE_SIZE: u64 = 4096;
pub const DMABUF_PAGE_SHIFT: u32 = 12;
pub const DMABUF_PAGE_MASK: u64 = !(DMABUF_PAGE_SIZE - 1);

/// Default physical DMA pool base (starts at 256 MiB mark in guest/physical RAM)
pub const GPU_DMA_PHYS_BASE: u64 = 0x1000_0000;
/// 32,768 pages * 4 KiB = 128 MiB managed GPU/DMA framebuffer arena
pub const GPU_DMA_TOTAL_PAGES: usize = 32_768;
pub const GPU_DMA_BITMAP_WORDS: usize = GPU_DMA_TOTAL_PAGES / 64; // 512 u64 words (4 KiB)

/// Maximum single DMA-BUF allocation: 32 MiB (8,192 pages) - fits 4K RGBA8888 (3840x2160x4)
pub const MAX_DMABUF_SIZE: u64 = 32 * 1024 * 1024;
pub const MAX_DMABUF_OBJECTS: usize = 64;
pub const MAX_SG_SEGMENTS: usize = 32;
pub const MAX_DMABUF_FDS: usize = 128;
pub const MAX_PROC_HANDLES: usize = 256;
pub const MAX_DMABUF_MAPPINGS: usize = 128;
pub const MAX_GRANTS_PER_OBJ: usize = 8;
pub const MAX_ATTACHMENTS_PER_OBJ: usize = 4;

/// Dedicated canonical 64-bit user virtual window for DMA-BUF mappings.
/// Cleanly separated from ELF image, stack, heap, and SHM regions.
pub const DMABUF_VIRT_BASE: u64 = 0x0000_6400_0000_0000;
pub const DMABUF_VIRT_END: u64 = 0x0000_6500_0000_0000;

/// Starting FD number for exported DMA-BUF PRIME descriptors
pub const DMABUF_FD_BASE: i32 = 100;

/// Wildcard target PID allowing any authorized receiver holding the FD
pub const DMABUF_ANY_PID: u32 = 0;

// Allocation / Usage Flags
pub const DMABUF_FLAG_SCANOUT: u32       = 1 << 0;
pub const DMABUF_FLAG_RENDER_TARGET: u32 = 1 << 1;
pub const DMABUF_FLAG_TEXTURE: u32       = 1 << 2;
pub const DMABUF_FLAG_CPU_ACCESS: u32    = 1 << 3;
pub const DMABUF_FLAG_CONTIGUOUS: u32    = 1 << 4;
pub const DMABUF_FLAG_ZERO_INIT: u32     = 1 << 5;

// Access Permissions
pub const DMABUF_PERM_READ: u32   = 1 << 0;
pub const DMABUF_PERM_WRITE: u32  = 1 << 1;
pub const DMABUF_PERM_EXPORT: u32 = 1 << 2;
pub const DMABUF_PERM_ALL: u32    = DMABUF_PERM_READ | DMABUF_PERM_WRITE | DMABUF_PERM_EXPORT;

// Virtual Memory Protection Bits (x86_64 Page Table compatible)
pub const DMABUF_PROT_READ: u32  = 1 << 0;
pub const DMABUF_PROT_WRITE: u32 = 1 << 1;
pub const DMABUF_PROT_WC: u32    = 1 << 2; // Write-Combining for GPU BAR / FB

// Hardware Page Table Entry Flags
pub const PTE_PRESENT: u64 = 1 << 0;
pub const PTE_WRITABLE: u64 = 1 << 1;
pub const PTE_USER: u64    = 1 << 2;
pub const PTE_PAT_WC: u64  = 1 << 3 | 1 << 7; // Write-Combining cache attribute
pub const PTE_SHM_BIT: u64 = 1 << 9;          // Available OS bit marking shared GPU frame

// Cache Coherency Sync Flags (Linux UAPI dma_buf_sync compatible)
pub const DMA_BUF_SYNC_READ: u32  = 1 << 0;
pub const DMA_BUF_SYNC_WRITE: u32 = 1 << 1;
pub const DMA_BUF_SYNC_RW: u32    = DMA_BUF_SYNC_READ | DMA_BUF_SYNC_WRITE;
pub const DMA_BUF_SYNC_START: u32 = 0 << 2;
pub const DMA_BUF_SYNC_END: u32   = 1 << 2;

// Negative POSIX Error Codes
pub const EPERM: i32  = -1;
pub const ENOENT: i32 = -2;
pub const EBADF: i32  = -9;
pub const ENOMEM: i32 = -12;
pub const EACCES: i32 = -13;
pub const EFAULT: i32 = -14;
pub const EBUSY: i32  = -16;
pub const EINVAL: i32 = -22;
pub const ENOSPC: i32 = -28;

// ============================================================================
// Pixel Formats & Hardware Attachment Types
// ============================================================================

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u32)]
pub enum DmaBufFormat {
    RawLinear = 0,
    Xrgb8888  = 0x34325258, // 'XR24'
    Argb8888  = 0x34325241, // 'AR24'
    Bgra8888  = 0x34324142, // 'BA24'
    Rgba8888  = 0x34324252, // 'RA24'
    Rgb565    = 0x36314752, // 'RG16'
    Nv12      = 0x3231564E, // 'NV12' (YUV 4:2:0 video playback)
}

impl DmaBufFormat {
    pub fn from_u32(val: u32) -> Option<Self> {
        match val {
            0 => Some(Self::RawLinear),
            0x34325258 => Some(Self::Xrgb8888),
            0x34325241 => Some(Self::Argb8888),
            0x34324142 => Some(Self::Bgra8888),
            0x34324252 => Some(Self::Rgba8888),
            0x36314752 => Some(Self::Rgb565),
            0x3231564E => Some(Self::Nv12),
            _ => None,
        }
    }

    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::Xrgb8888 | Self::Argb8888 | Self::Bgra8888 | Self::Rgba8888 => 4,
            Self::Rgb565 => 2,
            Self::Nv12 | Self::RawLinear => 1,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u32)]
pub enum DmaDeviceType {
    None         = 0,
    DrmScanout   = 1, // Direct CRTC / KMS Plane Scanout
    VirtioGpu3D  = 2, // VirtIO-GPU Virgl 3D Resource Backing
    VideoDecoder = 3, // Hardware Media / YUV Pipeline
}

impl DmaDeviceType {
    pub fn from_u32(val: u32) -> Option<Self> {
        match val {
            1 => Some(Self::DrmScanout),
            2 => Some(Self::VirtioGpu3D),
            3 => Some(Self::VideoDecoder),
            _ => None,
        }
    }
}

// ============================================================================
// Scatter-Gather Extent Table (sg_table)
// ============================================================================

/// Contiguous run of physical 4 KiB pages within the DMA pool
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(C)]
pub struct SgEntry {
    pub phys_addr: u64,
    pub page_count: u32,
    pub _pad: u32,
}

impl SgEntry {
    pub const fn empty() -> Self {
        Self {
            phys_addr: 0,
            page_count: 0,
            _pad: 0,
        }
    }

    #[inline]
    pub const fn byte_len(&self) -> u64 {
        (self.page_count as u64) * DMABUF_PAGE_SIZE
    }
}

/// Fixed-capacity scatter-gather list tracking physical DMA extents without heap alloc
#[derive(Copy, Clone)]
pub struct SgTable {
    pub entries: [SgEntry; MAX_SG_SEGMENTS],
    pub count: usize,
    pub total_pages: u32,
}

impl SgTable {
    pub const fn empty() -> Self {
        Self {
            entries: [SgEntry::empty(); MAX_SG_SEGMENTS],
            count: 0,
            total_pages: 0,
        }
    }

    pub fn clear(&mut self) {
        self.count = 0;
        self.total_pages = 0;
    }

    /// Appends a contiguous run, coalescing with the previous segment if adjacent
    pub fn push_run(&mut self, phys_addr: u64, page_count: u32) -> Result<(), i32> {
        if page_count == 0 {
            return Ok(());
        }
        if self.count > 0 {
            let last = &mut self.entries[self.count - 1];
            if last.phys_addr + last.byte_len() == phys_addr {
                last.page_count += page_count;
                self.total_pages += page_count;
                return Ok(());
            }
        }
        if self.count >= MAX_SG_SEGMENTS {
            return Err(ENOSPC);
        }
        self.entries[self.count] = SgEntry {
            phys_addr,
            page_count,
            _pad: 0,
        };
        self.count += 1;
        self.total_pages += page_count;
        Ok(())
    }

    /// Resolves the physical frame address for linear page index `page_idx`
    pub fn phys_for_page(&self, mut page_idx: u32) -> Option<u64> {
        if page_idx >= self.total_pages {
            return None;
        }
        for i in 0..self.count {
            let entry = &self.entries[i];
            if page_idx < entry.page_count {
                return Some(entry.phys_addr + (page_idx as u64) * DMABUF_PAGE_SIZE);
            }
            page_idx -= entry.page_count;
        }
        None
    }

    #[inline]
    pub const fn is_contiguous(&self) -> bool {
        self.count <= 1
    }

    #[inline]
    pub const fn base_phys(&self) -> u64 {
        if self.count == 0 {
            0
        } else {
            self.entries[0].phys_addr
        }
    }
}

// ============================================================================
// GPU Physical Frame Pool Allocator (Bit-Packed Extent & Buddy-Style Search)
// ============================================================================

/// Pure `#![no_std]` physical GPU/DMA page allocator over a 128 MiB pool.
/// Tracks page occupancy using 512 64-bit words (1 bit per 4 KiB page).
pub struct GpuMemoryAllocator {
    pub base_phys: u64,
    pub total_pages: usize,
    pub free_pages: usize,
    pub peak_used_pages: usize,
    pub alloc_calls: u64,
    pub free_calls: u64,
    pub contiguous_allocs: u64,
    pub scatter_gather_allocs: u64,
    bitmap: [u64; GPU_DMA_BITMAP_WORDS],
}

impl GpuMemoryAllocator {
    pub const fn new(base_phys: u64) -> Self {
        Self {
            base_phys,
            total_pages: GPU_DMA_TOTAL_PAGES,
            free_pages: GPU_DMA_TOTAL_PAGES,
            peak_used_pages: 0,
            alloc_calls: 0,
            free_calls: 0,
            contiguous_allocs: 0,
            scatter_gather_allocs: 0,
            bitmap: [0u64; GPU_DMA_BITMAP_WORDS],
        }
    }

    pub fn reset(&mut self, base_phys: u64) {
        self.base_phys = base_phys;
        self.total_pages = GPU_DMA_TOTAL_PAGES;
        self.free_pages = GPU_DMA_TOTAL_PAGES;
        self.peak_used_pages = 0;
        self.alloc_calls = 0;
        self.free_calls = 0;
        self.contiguous_allocs = 0;
        self.scatter_gather_allocs = 0;
        for w in self.bitmap.iter_mut() {
            *w = 0;
        }
    }

    #[inline]
    fn is_page_used(&self, page_idx: usize) -> bool {
        let word = page_idx / 64;
        let bit = page_idx % 64;
        ((self.bitmap[word] >> bit) & 1) != 0
    }

    #[inline]
    fn set_page_used(&mut self, page_idx: usize, used: bool) {
        let word = page_idx / 64;
        let bit = page_idx % 64;
        if used {
            self.bitmap[word] |= 1u64 << bit;
        } else {
            self.bitmap[word] &= !(1u64 << bit);
        }
    }

    #[inline]
    pub fn used_pages(&self) -> usize {
        self.total_pages - self.free_pages
    }

    fn update_peak(&mut self) {
        let used = self.used_pages();
        if used > self.peak_used_pages {
            self.peak_used_pages = used;
        }
    }

    /// Attempts to allocate `needed_pages` as a single physically contiguous run
    pub fn alloc_contiguous(&mut self, needed_pages: u32, sg: &mut SgTable) -> Result<(), i32> {
        let need = needed_pages as usize;
        if need == 0 || need > self.free_pages {
            return Err(ENOMEM);
        }

        let mut run_start = 0usize;
        let mut run_len = 0usize;
        let mut word_idx = 0usize;

        while word_idx < GPU_DMA_BITMAP_WORDS {
            let word = self.bitmap[word_idx];
            if word == u64::MAX {
                run_len = 0;
                word_idx += 1;
                continue;
            }
            if word == 0 && run_len == 0 {
                // Fast-path skip over completely free 64-page words
                run_start = word_idx * 64;
                run_len = 64;
                if run_len >= need {
                    break;
                }
                word_idx += 1;
                continue;
            }
            if word == 0 && run_len > 0 {
                run_len += 64;
                if run_len >= need {
                    break;
                }
                word_idx += 1;
                continue;
            }

            // Mixed word: check individual bits
            let base = word_idx * 64;
            for b in 0..64 {
                if ((word >> b) & 1) == 0 {
                    if run_len == 0 {
                        run_start = base + b;
                    }
                    run_len += 1;
                    if run_len >= need {
                        break;
                    }
                } else {
                    run_len = 0;
                }
            }
            if run_len >= need {
                break;
            }
            word_idx += 1;
        }

        if run_len < need {
            return Err(ENOMEM);
        }

        // Mark run allocated
        for p in run_start..(run_start + need) {
            self.set_page_used(p, true);
        }
        self.free_pages -= need;
        self.alloc_calls += 1;
        self.contiguous_allocs += 1;
        self.update_peak();

        let phys = self.base_phys + (run_start as u64) * DMABUF_PAGE_SIZE;
        sg.clear();
        sg.push_run(phys, needed_pages)
    }

    /// Allocates `needed_pages` across up to `MAX_SG_SEGMENTS` scatter-gather runs
    pub fn alloc_scatter_gather(&mut self, needed_pages: u32, sg: &mut SgTable) -> Result<(), i32> {
        let mut remaining = needed_pages as usize;
        if remaining == 0 || remaining > self.free_pages {
            return Err(ENOMEM);
        }

        // First try fast contiguous allocation; works cleanly when unfragmented
        sg.clear();
        if self.alloc_contiguous(needed_pages, sg).is_ok() {
            return Ok(());
        }

        // Fallback: harvest largest available free runs into SgTable
        sg.clear();
        let mut idx = 0usize;
        while idx < self.total_pages && remaining > 0 {
            if self.is_page_used(idx) {
                idx += 1;
                continue;
            }
            let start = idx;
            let mut count = 0usize;
            while idx < self.total_pages && !self.is_page_used(idx) && count < remaining {
                count += 1;
                idx += 1;
            }
            let phys = self.base_phys + (start as u64) * DMABUF_PAGE_SIZE;
            if let Err(e) = sg.push_run(phys, count as u32) {
                // Roll back any partial allocations on failure
                self.free_sg_table(sg);
                return Err(e);
            }
            for p in start..(start + count) {
                self.set_page_used(p, true);
            }
            self.free_pages -= count;
            remaining -= count;
        }

        if remaining > 0 {
            self.free_sg_table(sg);
            return Err(ENOMEM);
        }

        self.alloc_calls += 1;
        self.scatter_gather_allocs += 1;
        self.update_peak();
        Ok(())
    }

    /// Returns all physical frames referenced by `sg` back to the free pool
    pub fn free_sg_table(&mut self, sg: &mut SgTable) {
        if sg.total_pages == 0 {
            sg.clear();
            return;
        }
        for i in 0..sg.count {
            let entry = sg.entries[i];
            if entry.phys_addr < self.base_phys {
                continue;
            }
            let start_page = ((entry.phys_addr - self.base_phys) / DMABUF_PAGE_SIZE) as usize;
            let end_page = start_page + (entry.page_count as usize);
            if end_page <= self.total_pages {
                for p in start_page..end_page {
                    if self.is_page_used(p) {
                        self.set_page_used(p, false);
                        self.free_pages += 1;
                    }
                }
            }
        }
        self.free_calls += 1;
        sg.clear();
    }
}

// ============================================================================
// DMA-BUF Object, ACL Grants, Device Attachments, and Virtual Mappings
// ============================================================================

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct DmaBufGrant {
    pub active: bool,
    pub target_pid: u32,
    pub rights: u32,
}

impl DmaBufGrant {
    pub const fn empty() -> Self {
        Self {
            active: false,
            target_pid: 0,
            rights: 0,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct DmaDeviceAttachment {
    pub active: bool,
    pub dev_type: DmaDeviceType,
    pub device_handle: u32, // e.g., DRM fb_id or Virgl resource_id
}

impl DmaDeviceAttachment {
    pub const fn empty() -> Self {
        Self {
            active: false,
            dev_type: DmaDeviceType::None,
            device_handle: 0,
        }
    }
}

/// Kernel representation of a single shareable zero-copy DMA-BUF object
pub struct DmaBufObject {
    pub id: u32,
    pub active: bool,
    pub owner_pid: u32,
    pub width: u32,
    pub height: u32,
    pub bpp: u32,
    pub stride: u32,
    pub format: DmaBufFormat,
    pub requested_size: u64,
    pub aligned_size: u64,
    pub flags: u32,
    pub refcount: u32,
    pub map_count: u32,
    pub sg_table: SgTable,
    pub sync_active: bool,
    pub sync_flags: u32,
    pub sync_count: u64,
    pub grants: [DmaBufGrant; MAX_GRANTS_PER_OBJ],
    pub attachments: [DmaDeviceAttachment; MAX_ATTACHMENTS_PER_OBJ],
}

impl DmaBufObject {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            active: false,
            owner_pid: 0,
            width: 0,
            height: 0,
            bpp: 0,
            stride: 0,
            format: DmaBufFormat::RawLinear,
            requested_size: 0,
            aligned_size: 0,
            flags: 0,
            refcount: 0,
            map_count: 0,
            sg_table: SgTable::empty(),
            sync_active: false,
            sync_flags: 0,
            sync_count: 0,
            grants: [DmaBufGrant::empty(); MAX_GRANTS_PER_OBJ],
            attachments: [DmaDeviceAttachment::empty(); MAX_ATTACHMENTS_PER_OBJ],
        }
    }

    pub fn grant_rights(&mut self, target_pid: u32, rights: u32) -> Result<(), i32> {
        for g in self.grants.iter_mut() {
            if g.active && g.target_pid == target_pid {
                g.rights |= rights;
                return Ok(());
            }
        }
        for g in self.grants.iter_mut() {
            if !g.active {
                g.active = true;
                g.target_pid = target_pid;
                g.rights = rights;
                return Ok(());
            }
        }
        Err(ENOSPC)
    }

    pub fn rights_for(&self, pid: u32) -> u32 {
        if pid == self.owner_pid {
            return DMABUF_PERM_ALL;
        }
        let mut combined = 0u32;
        for g in self.grants.iter() {
            if g.active && (g.target_pid == pid || g.target_pid == DMABUF_ANY_PID) {
                combined |= g.rights;
            }
        }
        combined
    }
}

/// Exported file descriptor / PRIME handle linking a shareable FD to a DMA-BUF object
#[derive(Copy, Clone)]
pub struct DmaBufFdEntry {
    pub fd: i32,
    pub active: bool,
    pub obj_idx: usize,
    pub exporter_pid: u32,
    pub target_pid: u32,
    pub rights: u32,
}

impl DmaBufFdEntry {
    pub const fn empty() -> Self {
        Self {
            fd: 0,
            active: false,
            obj_idx: 0,
            exporter_pid: 0,
            target_pid: 0,
            rights: 0,
        }
    }
}

/// Per-process GEM/DMA-BUF local buffer handle entry
#[derive(Copy, Clone)]
pub struct ProcHandleEntry {
    pub active: bool,
    pub pid: u32,
    pub handle: u32,
    pub obj_idx: usize,
    pub rights: u32,
}

impl ProcHandleEntry {
    pub const fn empty() -> Self {
        Self {
            active: false,
            pid: 0,
            handle: 0,
            obj_idx: 0,
            rights: 0,
        }
    }
}

/// Active virtual memory mapping of a DMA-BUF object into a process address space
#[derive(Copy, Clone)]
pub struct DmaBufMapping {
    pub active: bool,
    pub pid: u32,
    pub obj_idx: usize,
    pub virt_addr: u64,
    pub mapped_size: u64,
    pub guard_page_addr: u64,
    pub page_count: u32,
    pub prot: u32,
    pub pte_flags: u64,
}

impl DmaBufMapping {
    pub const fn empty() -> Self {
        Self {
            active: false,
            pid: 0,
            obj_idx: 0,
            virt_addr: 0,
            mapped_size: 0,
            guard_page_addr: 0,
            page_count: 0,
            prot: 0,
            pte_flags: 0,
        }
    }
}

// ============================================================================
// Return Payloads & Telemetry Diagnostics
// ============================================================================

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(C)]
pub struct DmaBufAllocResult {
    pub handle: u32,
    pub obj_id: u32,
    pub aligned_size: u64,
    pub stride: u32,
    pub phys_base: u64,
    pub sg_count: u32,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(C)]
pub struct DmaBufImportResult {
    pub handle: u32,
    pub obj_id: u32,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(C)]
pub struct DmaBufMapResult {
    pub virt_addr: u64,
    pub mapped_size: u64,
    pub guard_page_addr: u64,
    pub map_count: u32,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(C)]
pub struct DmaBufInfo {
    pub obj_id: u32,
    pub owner_pid: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
    pub size: u64,
    pub phys_base: u64,
    pub refcount: u32,
    pub map_count: u32,
    pub sg_count: u32,
    pub is_contiguous: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(C)]
pub struct ZeroCopyProof {
    pub verified_pages: u32,
    pub shared_bytes: u64,
    pub phys_base: u64,
    pub guard_verified: bool,
    pub is_zero_copy: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(C)]
pub struct DmaBufStats {
    pub active_objects: u32,
    pub active_fds: u32,
    pub active_mappings: u32,
    pub total_pool_pages: usize,
    pub free_pool_pages: usize,
    pub allocated_pool_pages: usize,
    pub peak_used_pages: usize,
    pub total_exports: u64,
    pub total_imports: u64,
    pub total_syncs: u64,
    pub zero_copy_frames_shared: u64,
}

// ============================================================================
// Central DMA-BUF Subsystem Controller
// ============================================================================

pub struct DmaBufManager {
    pub initialized: bool,
    pub allocator: GpuMemoryAllocator,
    pub objects: [DmaBufObject; MAX_DMABUF_OBJECTS],
    pub fds: [DmaBufFdEntry; MAX_DMABUF_FDS],
    pub proc_handles: [ProcHandleEntry; MAX_PROC_HANDLES],
    pub mappings: [DmaBufMapping; MAX_DMABUF_MAPPINGS],
    next_obj_id: u32,
    next_handle: u32,
    next_fd: i32,
    pub total_exports: u64,
    pub total_imports: u64,
    pub total_syncs: u64,
    pub zero_copy_frames_shared: u64,
}

impl DmaBufManager {
    pub const fn new() -> Self {
        const EMPTY_OBJ: DmaBufObject = DmaBufObject::empty();
        Self {
            initialized: false,
            allocator: GpuMemoryAllocator::new(GPU_DMA_PHYS_BASE),
            objects: [EMPTY_OBJ; MAX_DMABUF_OBJECTS],
            fds: [DmaBufFdEntry::empty(); MAX_DMABUF_FDS],
            proc_handles: [ProcHandleEntry::empty(); MAX_PROC_HANDLES],
            mappings: [DmaBufMapping::empty(); MAX_DMABUF_MAPPINGS],
            next_obj_id: 1,
            next_handle: 1,
            next_fd: DMABUF_FD_BASE,
            total_exports: 0,
            total_imports: 0,
            total_syncs: 0,
            zero_copy_frames_shared: 0,
        }
    }

    pub fn init(&mut self) {
        self.allocator.reset(GPU_DMA_PHYS_BASE);
        for o in self.objects.iter_mut() {
            *o = DmaBufObject::empty();
        }
        for f in self.fds.iter_mut() {
            *f = DmaBufFdEntry::empty();
        }
        for h in self.proc_handles.iter_mut() {
            *h = ProcHandleEntry::empty();
        }
        for m in self.mappings.iter_mut() {
            *m = DmaBufMapping::empty();
        }
        self.next_obj_id = 1;
        self.next_handle = 1;
        self.next_fd = DMABUF_FD_BASE;
        self.total_exports = 0;
        self.total_imports = 0;
        self.total_syncs = 0;
        self.zero_copy_frames_shared = 0;
        self.initialized = true;
    }

    #[inline]
    pub const fn page_align_up(size: u64) -> u64 {
        (size + DMABUF_PAGE_SIZE - 1) & DMABUF_PAGE_MASK
    }

    fn find_handle(&self, pid: u32, handle: u32) -> Option<usize> {
        for (idx, h) in self.proc_handles.iter().enumerate() {
            if h.active && h.pid == pid && h.handle == handle {
                return Some(idx);
            }
        }
        None
    }

    fn alloc_proc_handle(&mut self, pid: u32, obj_idx: usize, rights: u32) -> Result<u32, i32> {
        for slot in self.proc_handles.iter_mut() {
            if !slot.active {
                let h = self.next_handle;
                self.next_handle = self.next_handle.wrapping_add(1).max(1);
                slot.active = true;
                slot.pid = pid;
                slot.handle = h;
                slot.obj_idx = obj_idx;
                slot.rights = rights;
                return Ok(h);
            }
        }
        Err(ENOSPC)
    }

    /// Selects a non-overlapping canonical user virtual address in `pid`'s DMA-BUF window,
    /// reserving an extra trailing guard page (`DMABUF_PAGE_SIZE`) after the mapping.
    fn pick_user_vaddr(&self, pid: u32, aligned_size: u64) -> Result<u64, i32> {
        let span = aligned_size + DMABUF_PAGE_SIZE; // +1 unmapped guard page
        let mut candidate = DMABUF_VIRT_BASE;

        while candidate + span <= DMABUF_VIRT_END {
            let cand_end = candidate + span;
            let mut conflict = false;
            let mut max_conflict_end = candidate;

            for m in self.mappings.iter() {
                if m.active && m.pid == pid {
                    let m_start = m.virt_addr;
                    let m_end = m.virt_addr + m.mapped_size + DMABUF_PAGE_SIZE;
                    if candidate < m_end && cand_end > m_start {
                        conflict = true;
                        if m_end > max_conflict_end {
                            max_conflict_end = m_end;
                        }
                    }
                }
            }

            if !conflict {
                return Ok(candidate);
            }
            candidate = Self::page_align_up(max_conflict_end);
        }

        Err(ENOMEM)
    }

    /// Decrements object refcount and frees backing physical DMA frames if it reaches 0
    fn release_object_ref(&mut self, obj_idx: usize) {
        if obj_idx >= MAX_DMABUF_OBJECTS {
            return;
        }
        let obj = &mut self.objects[obj_idx];
        if !obj.active {
            return;
        }
        if obj.refcount > 0 {
            obj.refcount -= 1;
        }
        if obj.refcount == 0 {
            self.allocator.free_sg_table(&mut obj.sg_table);
            *obj = DmaBufObject::empty();
        }
    }

    /// Allocates a new hardware-backed DMA-BUF framebuffer / GPU buffer for `pid`.
    pub fn alloc(
        &mut self,
        pid: u32,
        width: u32,
        height: u32,
        format: DmaBufFormat,
        explicit_size: u64,
        flags: u32,
    ) -> Result<DmaBufAllocResult, i32> {
        if !self.initialized {
            self.init();
        }
        if pid == 0 {
            return Err(EINVAL);
        }

        let bpp = format.bytes_per_pixel();
        let (stride, raw_size) = if format == DmaBufFormat::RawLinear {
            if explicit_size == 0 {
                return Err(EINVAL);
            }
            (width, explicit_size)
        } else if format == DmaBufFormat::Nv12 {
            if width == 0 || height == 0 {
                return Err(EINVAL);
            }
            // NV12: Y plane (w * h) + interleaved UV plane (w * h / 2)
            let sz = (width as u64) * (height as u64) * 3 / 2;
            let req = if explicit_size > sz { explicit_size } else { sz };
            (width, req)
        } else {
            if width == 0 || height == 0 {
                return Err(EINVAL);
            }
            // 64-byte aligned scanline pitch for GPU tiling / DMA burst alignment
            let min_stride = width.checked_mul(bpp).ok_or(EINVAL)?;
            let aligned_stride = (min_stride + 63) & !63;
            let calc_size = (aligned_stride as u64) * (height as u64);
            let req = if explicit_size > calc_size {
                explicit_size
            } else {
                calc_size
            };
            (aligned_stride, req)
        };

        if raw_size == 0 || raw_size > MAX_DMABUF_SIZE {
            return Err(EINVAL);
        }

        let aligned_size = Self::page_align_up(raw_size);
        let needed_pages = (aligned_size / DMABUF_PAGE_SIZE) as u32;

        // Find free DmaBufObject slot
        let mut free_obj_idx = None;
        for (idx, obj) in self.objects.iter().enumerate() {
            if !obj.active {
                free_obj_idx = Some(idx);
                break;
            }
        }
        let obj_idx = free_obj_idx.ok_or(ENOSPC)?;

        // Reserve physical frames from GPU DMA pool
        let mut sg = SgTable::empty();
        let require_contig = (flags & (DMABUF_FLAG_CONTIGUOUS | DMABUF_FLAG_SCANOUT)) != 0;
        if require_contig {
            self.allocator.alloc_contiguous(needed_pages, &mut sg)?;
        } else {
            self.allocator.alloc_scatter_gather(needed_pages, &mut sg)?;
        }

        // Create local GEM/DMA-BUF handle for owner
        let handle = match self.alloc_proc_handle(pid, obj_idx, DMABUF_PERM_ALL) {
            Ok(h) => h,
            Err(e) => {
                self.allocator.free_sg_table(&mut sg);
                return Err(e);
            }
        };

        let obj_id = self.next_obj_id;
        self.next_obj_id = self.next_obj_id.wrapping_add(1).max(1);

        let phys_base = sg.base_phys();
        let sg_count = sg.count as u32;

        let obj = &mut self.objects[obj_idx];
        *obj = DmaBufObject::empty();
        obj.id = obj_id;
        obj.active = true;
        obj.owner_pid = pid;
        obj.width = width;
        obj.height = height;
        obj.bpp = bpp * 8;
        obj.stride = stride;
        obj.format = format;
        obj.requested_size = raw_size;
        obj.aligned_size = aligned_size;
        obj.flags = flags;
        obj.refcount = 1; // Held by owner's local handle
        obj.map_count = 0;
        obj.sg_table = sg;

        Ok(DmaBufAllocResult {
            handle,
            obj_id,
            aligned_size,
            stride,
            phys_base,
            sg_count,
        })
    }

    /// Exports a local DMA-BUF handle into a shareable cross-process `dmabuf_fd` (PRIME handle-to-fd)
    pub fn export(
        &mut self,
        caller_pid: u32,
        handle: u32,
        target_pid: u32,
        rights: u32,
    ) -> Result<i32, i32> {
        let h_idx = self.find_handle(caller_pid, handle).ok_or(EBADF)?;
        let h_entry = self.proc_handles[h_idx];
        if (h_entry.rights & DMABUF_PERM_EXPORT) == 0 {
            return Err(EACCES);
        }

        let effective_rights = rights & (h_entry.rights | DMABUF_PERM_READ);
        if (effective_rights & DMABUF_PERM_READ) == 0 {
            return Err(EINVAL);
        }

        let obj_idx = h_entry.obj_idx;
        if !self.objects[obj_idx].active {
            return Err(ENOENT);
        }

        // Grant ACL access to target_pid
        self.objects[obj_idx].grant_rights(target_pid, effective_rights)?;

        // Allocate shareable FD entry
        for fd_slot in self.fds.iter_mut() {
            if !fd_slot.active {
                let fd = self.next_fd;
                self.next_fd += 1;
                fd_slot.fd = fd;
                fd_slot.active = true;
                fd_slot.obj_idx = obj_idx;
                fd_slot.exporter_pid = caller_pid;
                fd_slot.target_pid = target_pid;
                fd_slot.rights = effective_rights;

                self.objects[obj_idx].refcount += 1;
                self.total_exports += 1;
                return Ok(fd);
            }
        }
        Err(ENOSPC)
    }

    /// Imports a shareable `dmabuf_fd` into the caller's (e.g. compositor) address/handle table
    pub fn import(&mut self, caller_pid: u32, dmabuf_fd: i32) -> Result<DmaBufImportResult, i32> {
        if caller_pid == 0 {
            return Err(EINVAL);
        }

        let mut found_fd = None;
        for f in self.fds.iter() {
            if f.active && f.fd == dmabuf_fd {
                found_fd = Some(*f);
                break;
            }
        }
        let fd_entry = found_fd.ok_or(EBADF)?;

        // Enforce strict target PID capability isolation unless wildcard or owner
        if fd_entry.target_pid != DMABUF_ANY_PID
            && fd_entry.target_pid != caller_pid
            && fd_entry.exporter_pid != caller_pid
        {
            return Err(EACCES);
        }

        let obj_idx = fd_entry.obj_idx;
        if !self.objects[obj_idx].active {
            return Err(ENOENT);
        }

        let rights = if caller_pid == self.objects[obj_idx].owner_pid {
            DMABUF_PERM_ALL
        } else {
            fd_entry.rights
        };

        let new_handle = self.alloc_proc_handle(caller_pid, obj_idx, rights)?;
        self.objects[obj_idx].refcount += 1;
        self.total_imports += 1;

        let obj = &self.objects[obj_idx];
        Ok(DmaBufImportResult {
            handle: new_handle,
            obj_id: obj.id,
            size: obj.aligned_size,
            width: obj.width,
            height: obj.height,
            stride: obj.stride,
            format: obj.format as u32,
        })
    }

    /// Maps the physical backing frames of a DMA-BUF into `caller_pid`'s virtual address space.
    /// Both client and compositor end up pointing directly to identical physical pages.
    pub fn map(
        &mut self,
        caller_pid: u32,
        handle: u32,
        prot: u32,
    ) -> Result<DmaBufMapResult, i32> {
        let h_idx = self.find_handle(caller_pid, handle).ok_or(EBADF)?;
        let h_entry = self.proc_handles[h_idx];
        let obj_idx = h_entry.obj_idx;

        if !self.objects[obj_idx].active {
            return Err(ENOENT);
        }

        // Permission enforcement against handle's granted rights
        if (prot & DMABUF_PROT_READ) != 0 && (h_entry.rights & DMABUF_PERM_READ) == 0 {
            return Err(EACCES);
        }
        if (prot & DMABUF_PROT_WRITE) != 0 && (h_entry.rights & DMABUF_PERM_WRITE) == 0 {
            return Err(EACCES);
        }

        let mapped_size = self.objects[obj_idx].aligned_size;
        let page_count = self.objects[obj_idx].sg_table.total_pages;
        let virt_addr = self.pick_user_vaddr(caller_pid, mapped_size)?;
        let guard_page_addr = virt_addr + mapped_size;

        let mut pte_flags = PTE_PRESENT | PTE_USER | PTE_SHM_BIT;
        if (prot & DMABUF_PROT_WRITE) != 0 {
            pte_flags |= PTE_WRITABLE;
        }
        if (prot & DMABUF_PROT_WC) != 0 {
            pte_flags |= PTE_PAT_WC;
        }

        let mut free_map_idx = None;
        for (i, m) in self.mappings.iter().enumerate() {
            if !m.active {
                free_map_idx = Some(i);
                break;
            }
        }
        let map_idx = free_map_idx.ok_or(ENOSPC)?;

        self.mappings[map_idx] = DmaBufMapping {
            active: true,
            pid: caller_pid,
            obj_idx,
            virt_addr,
            mapped_size,
            guard_page_addr,
            page_count,
            prot,
            pte_flags,
        };

        self.objects[obj_idx].map_count += 1;
        self.objects[obj_idx].refcount += 1;
        if self.objects[obj_idx].map_count > 1 {
            self.zero_copy_frames_shared += page_count as u64;
        }

        Ok(DmaBufMapResult {
            virt_addr,
            mapped_size,
            guard_page_addr,
            map_count: self.objects[obj_idx].map_count,
        })
    }

    /// Unmaps a virtual DMA-BUF mapping for `caller_pid`, dropping its reference
    pub fn unmap(&mut self, caller_pid: u32, virt_addr: u64) -> Result<(), i32> {
        let mut target_idx = None;
        for (i, m) in self.mappings.iter().enumerate() {
            if m.active && m.pid == caller_pid && m.virt_addr == virt_addr {
                target_idx = Some(i);
                break;
            }
        }
        let idx = target_idx.ok_or(ENOENT)?;
        let obj_idx = self.mappings[idx].obj_idx;

        self.mappings[idx] = DmaBufMapping::empty();
        if self.objects[obj_idx].map_count > 0 {
            self.objects[obj_idx].map_count -= 1;
        }
        self.release_object_ref(obj_idx);
        Ok(())
    }

    /// Closes an exported `dmabuf_fd` and drops its reference to the underlying object
    pub fn close_fd(&mut self, caller_pid: u32, dmabuf_fd: i32) -> Result<(), i32> {
        let mut target_idx = None;
        for (i, f) in self.fds.iter().enumerate() {
            if f.active && f.fd == dmabuf_fd {
                if f.exporter_pid != caller_pid
                    && f.target_pid != caller_pid
                    && f.target_pid != DMABUF_ANY_PID
                {
                    return Err(EACCES);
                }
                target_idx = Some(i);
                break;
            }
        }
        let idx = target_idx.ok_or(EBADF)?;
        let obj_idx = self.fds[idx].obj_idx;
        self.fds[idx] = DmaBufFdEntry::empty();
        self.release_object_ref(obj_idx);
        Ok(())
    }

    /// Frees a process-local DMA-BUF handle, reclaiming physical frames only if `refcount == 0`
    pub fn free_handle(&mut self, caller_pid: u32, handle: u32) -> Result<(), i32> {
        let h_idx = self.find_handle(caller_pid, handle).ok_or(EBADF)?;
        let obj_idx = self.proc_handles[h_idx].obj_idx;
        self.proc_handles[h_idx] = ProcHandleEntry::empty();
        self.release_object_ref(obj_idx);
        Ok(())
    }

    /// Cache coherency barrier (`DMA_BUF_SYNC_START` / `DMA_BUF_SYNC_END`)
    pub fn sync(&mut self, caller_pid: u32, handle: u32, flags: u32) -> Result<(), i32> {
        let h_idx = self.find_handle(caller_pid, handle).ok_or(EBADF)?;
        let obj_idx = self.proc_handles[h_idx].obj_idx;
        if (flags & DMA_BUF_SYNC_RW) == 0 {
            return Err(EINVAL);
        }

        // Emit hardware memory fence on x86_64 when running on bare-metal / QEMU
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("mfence", options(nostack, preserves_flags));
        }

        let obj = &mut self.objects[obj_idx];
        obj.sync_flags = flags;
        obj.sync_active = (flags & DMA_BUF_SYNC_END) == 0;
        obj.sync_count += 1;
        self.total_syncs += 1;
        Ok(())
    }

    /// Binds a DMA-BUF directly to a hardware engine (e.g. DRM scanout CRTC or VirtIO-GPU 3D)
    pub fn attach_device(
        &mut self,
        caller_pid: u32,
        handle: u32,
        dev_type: DmaDeviceType,
        device_handle: u32,
    ) -> Result<(), i32> {
        if dev_type == DmaDeviceType::None || device_handle == 0 {
            return Err(EINVAL);
        }
        let h_idx = self.find_handle(caller_pid, handle).ok_or(EBADF)?;
        let obj_idx = self.proc_handles[h_idx].obj_idx;
        let obj = &mut self.objects[obj_idx];

        if dev_type == DmaDeviceType::DrmScanout && !obj.sg_table.is_contiguous() {
            // Hardware display CRTC requires contiguous scanout memory unless IOMMU/GTT is present
            if (obj.flags & DMABUF_FLAG_CONTIGUOUS) != 0 {
                return Err(EINVAL);
            }
        }

        for att in obj.attachments.iter_mut() {
            if att.active && att.dev_type == dev_type && att.device_handle == device_handle {
                return Err(EBUSY);
            }
        }
        for att in obj.attachments.iter_mut() {
            if !att.active {
                att.active = true;
                att.dev_type = dev_type;
                att.device_handle = device_handle;
                obj.refcount += 1;
                return Ok(());
            }
        }
        Err(ENOSPC)
    }

    /// Detaches a hardware engine from a DMA-BUF and decrements reference count
    pub fn detach_device(
        &mut self,
        caller_pid: u32,
        handle: u32,
        dev_type: DmaDeviceType,
        device_handle: u32,
    ) -> Result<(), i32> {
        let h_idx = self.find_handle(caller_pid, handle).ok_or(EBADF)?;
        let obj_idx = self.proc_handles[h_idx].obj_idx;
        let mut found = false;
        for att in self.objects[obj_idx].attachments.iter_mut() {
            if att.active && att.dev_type == dev_type && att.device_handle == device_handle {
                *att = DmaDeviceAttachment::empty();
                found = true;
                break;
            }
        }
        if !found {
            return Err(ENOENT);
        }
        self.release_object_ref(obj_idx);
        Ok(())
    }

    /// Cleans up all mappings, handles, and exported FDs owned by an exiting process
    /// while safely preserving buffers that remain imported/mapped by live peers (compositor).
    pub fn cleanup_process(&mut self, pid: u32) {
        for i in 0..MAX_DMABUF_MAPPINGS {
            if self.mappings[i].active && self.mappings[i].pid == pid {
                let obj_idx = self.mappings[i].obj_idx;
                self.mappings[i] = DmaBufMapping::empty();
                if self.objects[obj_idx].map_count > 0 {
                    self.objects[obj_idx].map_count -= 1;
                }
                self.release_object_ref(obj_idx);
            }
        }
        for i in 0..MAX_PROC_HANDLES {
            if self.proc_handles[i].active && self.proc_handles[i].pid == pid {
                let obj_idx = self.proc_handles[i].obj_idx;
                self.proc_handles[i] = ProcHandleEntry::empty();
                self.release_object_ref(obj_idx);
            }
        }
        for i in 0..MAX_DMABUF_FDS {
            if self.fds[i].active && self.fds[i].exporter_pid == pid {
                let obj_idx = self.fds[i].obj_idx;
                self.fds[i] = DmaBufFdEntry::empty();
                self.release_object_ref(obj_idx);
            }
        }
    }

    /// Queries metadata and reference state for a handle owned/imported by `caller_pid`
    pub fn info(&self, caller_pid: u32, handle: u32) -> Result<DmaBufInfo, i32> {
        let h_idx = self.find_handle(caller_pid, handle).ok_or(EBADF)?;
        let obj_idx = self.proc_handles[h_idx].obj_idx;
        let obj = &self.objects[obj_idx];
        if !obj.active {
            return Err(ENOENT);
        }
        Ok(DmaBufInfo {
            obj_id: obj.id,
            owner_pid: obj.owner_pid,
            width: obj.width,
            height: obj.height,
            stride: obj.stride,
            format: obj.format as u32,
            size: obj.aligned_size,
            phys_base: obj.sg_table.base_phys(),
            refcount: obj.refcount,
            map_count: obj.map_count,
            sg_count: obj.sg_table.count as u32,
            is_contiguous: obj.sg_table.is_contiguous(),
        })
    }

    /// Translates a mapped user virtual address in `pid` to `(phys_addr, pte_flags)`
    pub fn translate_user_vaddr(&self, pid: u32, vaddr: u64) -> Option<(u64, u64)> {
        let aligned_va = vaddr & DMABUF_PAGE_MASK;
        let in_page_off = vaddr & (DMABUF_PAGE_SIZE - 1);

        for m in self.mappings.iter() {
            if m.active && m.pid == pid {
                if aligned_va >= m.virt_addr && aligned_va < m.virt_addr + m.mapped_size {
                    let page_idx = ((aligned_va - m.virt_addr) / DMABUF_PAGE_SIZE) as u32;
                    let phys_page = self.objects[m.obj_idx].sg_table.phys_for_page(page_idx)?;
                    return Some((phys_page + in_page_off, m.pte_flags));
                }
            }
        }
        None
    }

    /// Verifies page-by-page that two distinct process mappings point to the EXACT SAME
    /// physical memory frames (100% zero-copy proof) with trailing guard pages unmapped.
    pub fn verify_zero_copy_sharing(
        &self,
        pid_a: u32,
        va_a: u64,
        pid_b: u32,
        va_b: u64,
    ) -> Result<ZeroCopyProof, i32> {
        let mut map_a = None;
        let mut map_b = None;
        for m in self.mappings.iter() {
            if m.active && m.pid == pid_a && m.virt_addr == va_a {
                map_a = Some(*m);
            }
            if m.active && m.pid == pid_b && m.virt_addr == va_b {
                map_b = Some(*m);
            }
        }
        let ma = map_a.ok_or(ENOENT)?;
        let mb = map_b.ok_or(ENOENT)?;

        if ma.obj_idx != mb.obj_idx || ma.page_count != mb.page_count {
            return Err(EINVAL);
        }

        let mut phys_base = 0u64;
        for page in 0..ma.page_count {
            let off = (page as u64) * DMABUF_PAGE_SIZE;
            let (pa, _) = self.translate_user_vaddr(pid_a, va_a + off).ok_or(EFAULT)?;
            let (pb, _) = self.translate_user_vaddr(pid_b, va_b + off).ok_or(EFAULT)?;
            if pa != pb {
                return Err(EFAULT);
            }
            if page == 0 {
                phys_base = pa;
            }
        }

        // Guard page verification: the trailing page immediately past each mapping must fault
        let guard_a_unmapped = self.translate_user_vaddr(pid_a, ma.guard_page_addr).is_none();
        let guard_b_unmapped = self.translate_user_vaddr(pid_b, mb.guard_page_addr).is_none();

        Ok(ZeroCopyProof {
            verified_pages: ma.page_count,
            shared_bytes: ma.mapped_size,
            phys_base,
            guard_verified: guard_a_unmapped && guard_b_unmapped,
            is_zero_copy: true,
        })
    }

    /// Global memory allocator and zero-copy sharing statistics
    pub fn stats(&self) -> DmaBufStats {
        let active_objects = self.objects.iter().filter(|o| o.active).count() as u32;
        let active_fds = self.fds.iter().filter(|f| f.active).count() as u32;
        let active_mappings = self.mappings.iter().filter(|m| m.active).count() as u32;

        DmaBufStats {
            active_objects,
            active_fds,
            active_mappings,
            total_pool_pages: self.allocator.total_pages,
            free_pool_pages: self.allocator.free_pages,
            allocated_pool_pages: self.allocator.used_pages(),
            peak_used_pages: self.allocator.peak_used_pages,
            total_exports: self.total_exports,
            total_imports: self.total_imports,
            total_syncs: self.total_syncs,
            zero_copy_frames_shared: self.zero_copy_frames_shared,
        }
    }
}

pub static mut GLOBAL_DMABUF: DmaBufManager = DmaBufManager::new();

// ============================================================================
// Unit Tests (Host Executable via rustc --test)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpu_allocator_contiguous_and_free() {
        let mut alloc = GpuMemoryAllocator::new(GPU_DMA_PHYS_BASE);
        let initial_free = alloc.free_pages;
        assert_eq!(initial_free, GPU_DMA_TOTAL_PAGES);

        let mut sg1 = SgTable::empty();
        // Allocate 1920x1080x4 bytes = 8,294,400 bytes -> 2025 pages contiguous
        alloc.alloc_contiguous(2025, &mut sg1).unwrap();
        assert!(sg1.is_contiguous());
        assert_eq!(sg1.total_pages, 2025);
        assert_eq!(sg1.base_phys(), GPU_DMA_PHYS_BASE);
        assert_eq!(alloc.free_pages, initial_free - 2025);

        let mut sg2 = SgTable::empty();
        alloc.alloc_contiguous(512, &mut sg2).unwrap();
        assert_eq!(sg2.base_phys(), GPU_DMA_PHYS_BASE + 2025 * DMABUF_PAGE_SIZE);

        alloc.free_sg_table(&mut sg1);
        alloc.free_sg_table(&mut sg2);
        assert_eq!(alloc.free_pages, initial_free);
    }

    #[test]
    fn test_zero_copy_client_to_compositor_sharing() {
        let mut mgr = DmaBufManager::new();
        mgr.init();

        let client_pid = 101;
        let compositor_pid = 2; // novadwm

        // Client allocates 1920x1080 ARGB8888 scanout buffer
        let alloc = mgr
            .alloc(
                client_pid,
                1920,
                1080,
                DmaBufFormat::Argb8888,
                0,
                DMABUF_FLAG_SCANOUT | DMABUF_FLAG_CPU_ACCESS,
            )
            .unwrap();
        assert_eq!(alloc.stride, 1920 * 4);
        assert_eq!(alloc.aligned_size, 8_294_400); // exact multiple of 4096

        // Client maps buffer Read/Write
        let client_map = mgr
            .map(client_pid, alloc.handle, DMABUF_PROT_READ | DMABUF_PROT_WRITE | DMABUF_PROT_WC)
            .unwrap();
        assert!(client_map.virt_addr >= DMABUF_VIRT_BASE);

        // Client exports FD exclusively for compositor
        let dmabuf_fd = mgr
            .export(client_pid, alloc.handle, compositor_pid, DMABUF_PERM_READ)
            .unwrap();
        assert!(dmabuf_fd >= DMABUF_FD_BASE);

        // Compositor imports FD and maps Read-Only
        let comp_import = mgr.import(compositor_pid, dmabuf_fd).unwrap();
        assert_eq!(comp_import.width, 1920);
        assert_eq!(comp_import.height, 1080);
        assert_eq!(comp_import.stride, 1920 * 4);

        let comp_map = mgr
            .map(compositor_pid, comp_import.handle, DMABUF_PROT_READ)
            .unwrap();

        // Proof: Every single 4KB page translates to the EXACT same physical frame!
        let proof = mgr
            .verify_zero_copy_sharing(client_pid, client_map.virt_addr, compositor_pid, comp_map.virt_addr)
            .unwrap();
        assert!(proof.is_zero_copy);
        assert!(proof.guard_verified);
        assert_eq!(proof.verified_pages, 2025);
        assert_eq!(proof.phys_base, alloc.phys_base);
    }

    #[test]
    fn test_permission_isolation_and_capability_checks() {
        let mut mgr = DmaBufManager::new();
        mgr.init();

        let client_pid = 50;
        let compositor_pid = 2;
        let attacker_pid = 666;

        let alloc = mgr
            .alloc(client_pid, 800, 600, DmaBufFormat::Xrgb8888, 0, DMABUF_FLAG_SCANOUT)
            .unwrap();
        let fd = mgr
            .export(client_pid, alloc.handle, compositor_pid, DMABUF_PERM_READ)
            .unwrap();

        // Unauthorized process cannot import FD restricted to compositor_pid
        assert_eq!(mgr.import(attacker_pid, fd), Err(EACCES));

        // Compositor imports with read-only rights; attempting PROT_WRITE map must fail
        let comp_imp = mgr.import(compositor_pid, fd).unwrap();
        assert_eq!(
            mgr.map(compositor_pid, comp_imp.handle, DMABUF_PROT_READ | DMABUF_PROT_WRITE),
            Err(EACCES)
        );
        // Compositor cannot re-export without DMABUF_PERM_EXPORT
        assert_eq!(
            mgr.export(compositor_pid, comp_imp.handle, attacker_pid, DMABUF_PERM_READ),
            Err(EACCES)
        );
    }

    #[test]
    fn test_refcounted_teardown_and_process_crash_resilience() {
        let mut mgr = DmaBufManager::new();
        mgr.init();
        let baseline_free = mgr.stats().free_pool_pages;

        let client_pid = 77;
        let compositor_pid = 2;

        let alloc = mgr
            .alloc(client_pid, 1280, 720, DmaBufFormat::Argb8888, 0, DMABUF_FLAG_RENDER_TARGET)
            .unwrap();
        let fd = mgr
            .export(client_pid, alloc.handle, compositor_pid, DMABUF_PERM_READ)
            .unwrap();
        let comp_imp = mgr.import(compositor_pid, fd).unwrap();
        let comp_map = mgr.map(compositor_pid, comp_imp.handle, DMABUF_PROT_READ).unwrap();

        // Client crashes / exits without unmapping or freeing!
        mgr.cleanup_process(client_pid);

        // Buffer MUST remain alive and readable in compositor!
        let comp_info = mgr.info(compositor_pid, comp_imp.handle).unwrap();
        assert!(comp_info.refcount >= 2); // compositor handle + compositor mapping
        assert!(mgr.translate_user_vaddr(compositor_pid, comp_map.virt_addr).is_some());

        // Compositor finishes frame: unmaps and closes handle -> refcount hits 0 -> 100% pages freed!
        mgr.unmap(compositor_pid, comp_map.virt_addr).unwrap();
        mgr.free_handle(compositor_pid, comp_imp.handle).unwrap();

        let final_stats = mgr.stats();
        assert_eq!(final_stats.active_objects, 0);
        assert_eq!(final_stats.active_mappings, 0);
        assert_eq!(final_stats.free_pool_pages, baseline_free);
    }

    #[test]
    fn test_device_attachment_and_sync_coherency() {
        let mut mgr = DmaBufManager::new();
        mgr.init();

        let pid = 42;
        let alloc = mgr
            .alloc(pid, 1920, 1080, DmaBufFormat::Nv12, 0, DMABUF_FLAG_CONTIGUOUS)
            .unwrap();

        // Attach to DRM scanout & VirtIO-GPU 3D
        mgr.attach_device(pid, alloc.handle, DmaDeviceType::DrmScanout, 10).unwrap();
        mgr.attach_device(pid, alloc.handle, DmaDeviceType::VirtioGpu3D, 201).unwrap();

        // Duplicate attachment rejected
        assert_eq!(
            mgr.attach_device(pid, alloc.handle, DmaDeviceType::DrmScanout, 10),
            Err(EBUSY)
        );

        // Cache sync start/end
        mgr.sync(pid, alloc.handle, DMA_BUF_SYNC_START | DMA_BUF_SYNC_WRITE).unwrap();
        mgr.sync(pid, alloc.handle, DMA_BUF_SYNC_END | DMA_BUF_SYNC_WRITE).unwrap();
        assert_eq!(mgr.stats().total_syncs, 2);

        // Drop local handle while attached -> object stays alive via device attachments!
        let ref_before = mgr.info(pid, alloc.handle).unwrap().refcount;
        assert_eq!(ref_before, 3); // handle + 2 attachments

        mgr.detach_device(pid, alloc.handle, DmaDeviceType::DrmScanout, 10).unwrap();
        mgr.detach_device(pid, alloc.handle, DmaDeviceType::VirtioGpu3D, 201).unwrap();
        mgr.free_handle(pid, alloc.handle).unwrap();
        assert_eq!(mgr.stats().active_objects, 0);
    }
}
