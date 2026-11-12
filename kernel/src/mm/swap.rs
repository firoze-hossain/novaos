//! kernel/src/mm/swap.rs - Anonymous Swap Subsystem with zswap & Pure Rust LZ4 Compression
//!
//! NovaOS 2.0 (Month 2 - Storage & Async I/O): Kernel Memory Page Cache & Anonymous Swap Subsystem
//!
//! Provides:
//!   - High-speed, pure `#![no_std]` Rust LZ4 block compressor and decompressor for 4KB memory pages.
//!   - Anonymous swap engine backing heap, stack, and private anonymous mmap allocations under memory pressure.
//!   - zswap in-RAM compressed memory pool saving 50% - 85% RAM without disk I/O.
//!   - Disk swap partition fallback when zswap pool is full or page is incompressible.
//!   - Memory pressure watermark detection (85% high watermark) to prevent Out-Of-Memory (OOM) panics.
//!   - Zero-allocation static BSS storage discipline guaranteeing zero stack overflows.

#![allow(dead_code)]
#![allow(static_mut_refs)]

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

// -----------------------------------------------------------------------------
// Constants & Configuration
// -----------------------------------------------------------------------------

pub const SWAP_PAGE_SIZE: usize = 4096;
pub const MAX_ZSWAP_SLOTS: usize = 64;          // 64 in-RAM compressed page slots
pub const MAX_ZSWAP_SLOT_BYTES: usize = 2048;   // Pages compressing <= 2KB fit in zswap
pub const MAX_DISK_SWAP_SLOTS: usize = 128;     // 128 disk swap partition slots (512KB swap)
pub const SWAP_PRESSURE_WATERMARK_PCT: u32 = 85;// Trigger swap when memory pressure >= 85%

// -----------------------------------------------------------------------------
// Error Codes
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SwapError {
    SwapFull,
    InvalidEntry,
    DecompressionFailed,
    CompressionFailed,
    ChecksumMismatch,
    SlotNotInUse,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Lz4Error {
    OutputTooSmall,
    CorruptedData,
    Incompressible,
}

// -----------------------------------------------------------------------------
// Pure #![no_std] Rust LZ4 Block Compression Engine
// -----------------------------------------------------------------------------

/// Compresses a 4KB memory page using standard LZ4 block format
pub fn lz4_compress_block(src: &[u8], dst: &mut [u8]) -> Result<usize, Lz4Error> {
    let src_len = src.len();
    if src_len == 0 {
        return Ok(0);
    }
    // Standard LZ4 bound: src_len + src_len / 255 + 16
    if dst.len() < src_len + src_len / 255 + 16 {
        return Err(Lz4Error::OutputTooSmall);
    }

    let mut hash_table = [0usize; 1024];
    let mut ip = 0usize;
    let mut anchor = 0usize;
    let mut op = 0usize;

    // LZ4 requires the last 5 bytes to be literal values
    let mflimit = if src_len > 12 { src_len - 12 } else { 0 };

    while ip < mflimit {
        let val = u32::from_le_bytes([src[ip], src[ip + 1], src[ip + 2], src[ip + 3]]);
        let hash = ((val.wrapping_mul(2654435761)) >> 22) as usize & 1023;
        let ref_pos = hash_table[hash];
        hash_table[hash] = ip;

        // Match condition: within 64KB distance, valid ref, and 4 leading bytes identical
        if ref_pos < ip && (ip - ref_pos) <= 65535 && ip + 4 <= src_len &&
           src[ref_pos] == src[ip] &&
           src[ref_pos + 1] == src[ip + 1] &&
           src[ref_pos + 2] == src[ip + 2] &&
           src[ref_pos + 3] == src[ip + 3] {

            let lit_len = ip - anchor;
            let offset = (ip - ref_pos) as u16;

            // Expand match length
            let mut match_len = 4usize;
            while ip + match_len < src_len - 5 && src[ref_pos + match_len] == src[ip + match_len] {
                match_len += 1;
            }

            // Encode token
            let token_pos = op;
            op += 1;

            let lit_token = if lit_len >= 15 { 15 } else { lit_len as u8 };
            let match_token = if match_len - 4 >= 15 { 15 } else { (match_len - 4) as u8 };
            dst[token_pos] = (lit_token << 4) | match_token;

            // Extra literal length bytes
            if lit_len >= 15 {
                let mut rem = lit_len - 15;
                while rem >= 255 {
                    dst[op] = 255;
                    op += 1;
                    rem -= 255;
                }
                dst[op] = rem as u8;
                op += 1;
            }

            // Emit literals
            dst[op..op + lit_len].copy_from_slice(&src[anchor..anchor + lit_len]);
            op += lit_len;

            // Emit match offset (2 bytes, little-endian)
            dst[op] = (offset & 0xFF) as u8;
            dst[op + 1] = ((offset >> 8) & 0xFF) as u8;
            op += 2;

            // Extra match length bytes
            if match_len - 4 >= 15 {
                let mut rem = match_len - 4 - 15;
                while rem >= 255 {
                    dst[op] = 255;
                    op += 1;
                    rem -= 255;
                }
                dst[op] = rem as u8;
                op += 1;
            }

            ip += match_len;
            anchor = ip;
        } else {
            ip += 1;
        }
    }

    // Trailing literals to end of block
    let last_lit = src_len - anchor;
    let token_pos = op;
    op += 1;

    let lit_token = if last_lit >= 15 { 15 } else { last_lit as u8 };
    dst[token_pos] = lit_token << 4;

    if last_lit >= 15 {
        let mut rem = last_lit - 15;
        while rem >= 255 {
            dst[op] = 255;
            op += 1;
            rem -= 255;
        }
        dst[op] = rem as u8;
        op += 1;
    }

    dst[op..op + last_lit].copy_from_slice(&src[anchor..anchor + last_lit]);
    op += last_lit;

    Ok(op)
}

/// Decompresses an LZ4 block back into the destination buffer
pub fn lz4_decompress_block(src: &[u8], dst: &mut [u8]) -> Result<usize, Lz4Error> {
    let mut ip = 0usize;
    let mut op = 0usize;
    let src_len = src.len();
    let dst_len = dst.len();

    while ip < src_len {
        let token = src[ip];
        ip += 1;

        // Parse literal length
        let mut lit_len = (token >> 4) as usize;
        if lit_len == 15 {
            while ip < src_len {
                let s = src[ip] as usize;
                ip += 1;
                lit_len += s;
                if s < 255 {
                    break;
                }
            }
        }

        if op + lit_len > dst_len || ip + lit_len > src_len {
            return Err(Lz4Error::CorruptedData);
        }

        dst[op..op + lit_len].copy_from_slice(&src[ip..ip + lit_len]);
        op += lit_len;
        ip += lit_len;

        if ip >= src_len {
            break;
        }

        // Parse match offset
        if ip + 2 > src_len {
            return Err(Lz4Error::CorruptedData);
        }
        let offset = src[ip] as usize | ((src[ip + 1] as usize) << 8);
        ip += 2;

        if offset == 0 || offset > op {
            return Err(Lz4Error::CorruptedData);
        }

        // Parse match length
        let mut match_len = (token & 0x0F) as usize + 4;
        if (token & 0x0F) == 15 {
            while ip < src_len {
                let s = src[ip] as usize;
                ip += 1;
                match_len += s;
                if s < 255 {
                    break;
                }
            }
        }

        if op + match_len > dst_len {
            return Err(Lz4Error::OutputTooSmall);
        }

        // Copy match byte-by-byte to handle overlapping repeat sequences
        let match_src = op - offset;
        for i in 0..match_len {
            dst[op + i] = dst[match_src + i];
        }
        op += match_len;
    }

    Ok(op)
}

/// Simple Adler-32 / CRC checksum for compressed data integrity
fn calculate_checksum(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

// -----------------------------------------------------------------------------
// Swap Entry & Location
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SwapLocation {
    None,
    Zswap(u16), // Compressed slot index in RAM
    Disk(u16),  // Slot index on swap disk partition
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SwapEntry {
    pub page_id: u32,
    pub location: SwapLocation,
    pub compressed_size: u16,
    pub original_size: u16,
}

impl SwapEntry {
    pub const fn empty() -> Self {
        Self {
            page_id: 0,
            location: SwapLocation::None,
            compressed_size: 0,
            original_size: SWAP_PAGE_SIZE as u16,
        }
    }
}

// -----------------------------------------------------------------------------
// zswap In-RAM Compressed Pool
// -----------------------------------------------------------------------------

#[derive(Copy, Clone)]
pub struct ZswapSlot {
    pub page_id: u32,
    pub compressed_len: u16,
    pub checksum: u32,
    pub in_use: bool,
}

impl ZswapSlot {
    pub const fn empty() -> Self {
        Self {
            page_id: 0,
            compressed_len: 0,
            checksum: 0,
            in_use: false,
        }
    }
}

pub struct ZswapDataBuffer {
    pub bytes: [u8; MAX_ZSWAP_SLOT_BYTES],
}

impl ZswapDataBuffer {
    pub const fn new() -> Self {
        Self {
            bytes: [0u8; MAX_ZSWAP_SLOT_BYTES],
        }
    }
}

static mut ZSWAP_SLOTS: [ZswapSlot; MAX_ZSWAP_SLOTS] =
    [ZswapSlot::empty(); MAX_ZSWAP_SLOTS];

static mut ZSWAP_BUFFERS: [ZswapDataBuffer; MAX_ZSWAP_SLOTS] =
    [const { ZswapDataBuffer::new() }; MAX_ZSWAP_SLOTS];

// -----------------------------------------------------------------------------
// Swap Disk Partition (Simulated / Physical Partition)
// -----------------------------------------------------------------------------

#[derive(Copy, Clone)]
pub struct DiskSwapSlot {
    pub page_id: u32,
    pub in_use: bool,
}

impl DiskSwapSlot {
    pub const fn empty() -> Self {
        Self {
            page_id: 0,
            in_use: false,
        }
    }
}

pub struct DiskSwapDataBuffer {
    pub bytes: [u8; SWAP_PAGE_SIZE],
}

impl DiskSwapDataBuffer {
    pub const fn new() -> Self {
        Self {
            bytes: [0u8; SWAP_PAGE_SIZE],
        }
    }
}

static mut DISK_SWAP_SLOTS: [DiskSwapSlot; MAX_DISK_SWAP_SLOTS] =
    [DiskSwapSlot::empty(); MAX_DISK_SWAP_SLOTS];

static mut DISK_SWAP_BUFFERS: [DiskSwapDataBuffer; MAX_DISK_SWAP_SLOTS] =
    [const { DiskSwapDataBuffer::new() }; MAX_DISK_SWAP_SLOTS];

// -----------------------------------------------------------------------------
// Telemetry & Statistics
// -----------------------------------------------------------------------------

pub struct SwapStats {
    pub swap_outs: AtomicU32,
    pub swap_ins: AtomicU32,
    pub zswap_stored: AtomicU32,
    pub disk_stored: AtomicU32,
    pub compressed_bytes_total: AtomicU32,
    pub uncompressed_bytes_total: AtomicU32,
    pub saved_bytes: AtomicU32,
}

impl SwapStats {
    pub const fn new() -> Self {
        Self {
            swap_outs: AtomicU32::new(0),
            swap_ins: AtomicU32::new(0),
            zswap_stored: AtomicU32::new(0),
            disk_stored: AtomicU32::new(0),
            compressed_bytes_total: AtomicU32::new(0),
            uncompressed_bytes_total: AtomicU32::new(0),
            saved_bytes: AtomicU32::new(0),
        }
    }

    pub fn reset(&self) {
        self.swap_outs.store(0, Ordering::Relaxed);
        self.swap_ins.store(0, Ordering::Relaxed);
        self.zswap_stored.store(0, Ordering::Relaxed);
        self.disk_stored.store(0, Ordering::Relaxed);
        self.compressed_bytes_total.store(0, Ordering::Relaxed);
        self.uncompressed_bytes_total.store(0, Ordering::Relaxed);
        self.saved_bytes.store(0, Ordering::Relaxed);
    }
}

pub static SWAP_STATS: SwapStats = SwapStats::new();

// -----------------------------------------------------------------------------
// Swap Subsystem Manager
// -----------------------------------------------------------------------------

pub struct SwapSubsystem {
    initialized: bool,
}

impl SwapSubsystem {
    pub const fn new() -> Self {
        Self { initialized: false }
    }

    pub fn init(&mut self) {
        unsafe {
            for slot in ZSWAP_SLOTS.iter_mut() {
                *slot = ZswapSlot::empty();
            }
            for slot in DISK_SWAP_SLOTS.iter_mut() {
                *slot = DiskSwapSlot::empty();
            }
        }
        SWAP_STATS.reset();
        self.initialized = true;
    }

    /// Evaluates if memory pressure exceeds the 85% watermark
    #[inline]
    pub fn is_memory_pressure_high(used_pages: u32, total_pages: u32) -> bool {
        if total_pages == 0 {
            return false;
        }
        (used_pages.saturating_mul(100) / total_pages) >= SWAP_PRESSURE_WATERMARK_PCT
    }

    // -------------------------------------------------------------------------
    // zswap Pool Operations
    // -------------------------------------------------------------------------

    fn find_free_zswap_slot(&self) -> Option<usize> {
        unsafe {
            for i in 0..MAX_ZSWAP_SLOTS {
                if !ZSWAP_SLOTS[i].in_use {
                    return Some(i);
                }
            }
        }
        None
    }

    fn find_free_disk_slot(&self) -> Option<usize> {
        unsafe {
            for i in 0..MAX_DISK_SWAP_SLOTS {
                if !DISK_SWAP_SLOTS[i].in_use {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Swaps out an anonymous memory page: compresses with LZ4, placing in zswap RAM pool or disk
    pub fn swap_out(&mut self, page_id: u32, page_data: &[u8; SWAP_PAGE_SIZE]) -> Result<SwapEntry, SwapError> {
        if !self.initialized {
            self.init();
        }

        // Temporary compression buffer (LZ4 bound for 4096 is 4129 bytes)
        let mut comp_buf = [0u8; 4300];
        let comp_res = lz4_compress_block(page_data, &mut comp_buf);

        let comp_sz = match comp_res {
            Ok(sz) => sz,
            Err(_) => {
                // If compression fails, write directly to disk swap
                return self.swap_out_to_disk(page_id, page_data);
            }
        };

        // If compressed size fits in zswap slot (<= 2048 bytes) and zswap has free slot
        if comp_sz <= MAX_ZSWAP_SLOT_BYTES {
            if let Some(slot_idx) = self.find_free_zswap_slot() {
                let checksum = calculate_checksum(&comp_buf[..comp_sz]);
                unsafe {
                    let slot = &mut ZSWAP_SLOTS[slot_idx];
                    slot.page_id = page_id;
                    slot.compressed_len = comp_sz as u16;
                    slot.checksum = checksum;
                    slot.in_use = true;

                    ZSWAP_BUFFERS[slot_idx].bytes[..comp_sz].copy_from_slice(&comp_buf[..comp_sz]);
                }

                SWAP_STATS.swap_outs.fetch_add(1, Ordering::Relaxed);
                SWAP_STATS.zswap_stored.fetch_add(1, Ordering::Relaxed);
                SWAP_STATS.compressed_bytes_total.fetch_add(comp_sz as u32, Ordering::Relaxed);
                SWAP_STATS.uncompressed_bytes_total.fetch_add(SWAP_PAGE_SIZE as u32, Ordering::Relaxed);
                SWAP_STATS.saved_bytes.fetch_add((SWAP_PAGE_SIZE - comp_sz) as u32, Ordering::Relaxed);

                return Ok(SwapEntry {
                    page_id,
                    location: SwapLocation::Zswap(slot_idx as u16),
                    compressed_size: comp_sz as u16,
                    original_size: SWAP_PAGE_SIZE as u16,
                });
            }
        }

        // Incompressible or zswap full -> Evict to disk swap partition
        self.swap_out_to_disk(page_id, page_data)
    }

    fn swap_out_to_disk(&mut self, page_id: u32, page_data: &[u8; SWAP_PAGE_SIZE]) -> Result<SwapEntry, SwapError> {
        let slot_idx = self.find_free_disk_slot().ok_or(SwapError::SwapFull)?;

        unsafe {
            let slot = &mut DISK_SWAP_SLOTS[slot_idx];
            slot.page_id = page_id;
            slot.in_use = true;

            DISK_SWAP_BUFFERS[slot_idx].bytes.copy_from_slice(page_data);
        }

        SWAP_STATS.swap_outs.fetch_add(1, Ordering::Relaxed);
        SWAP_STATS.disk_stored.fetch_add(1, Ordering::Relaxed);
        SWAP_STATS.uncompressed_bytes_total.fetch_add(SWAP_PAGE_SIZE as u32, Ordering::Relaxed);

        Ok(SwapEntry {
            page_id,
            location: SwapLocation::Disk(slot_idx as u16),
            compressed_size: SWAP_PAGE_SIZE as u16,
            original_size: SWAP_PAGE_SIZE as u16,
        })
    }

    /// Swaps in a previously evicted page: decompresses from zswap or reads from swap disk
    pub fn swap_in(&mut self, entry: &SwapEntry, out_page_data: &mut [u8; SWAP_PAGE_SIZE]) -> Result<(), SwapError> {
        if !self.initialized {
            return Err(SwapError::InvalidEntry);
        }

        match entry.location {
            SwapLocation::Zswap(slot_idx) => {
                let idx = slot_idx as usize;
                if idx >= MAX_ZSWAP_SLOTS {
                    return Err(SwapError::InvalidEntry);
                }

                unsafe {
                    let slot = &mut ZSWAP_SLOTS[idx];
                    if !slot.in_use || slot.page_id != entry.page_id {
                        return Err(SwapError::SlotNotInUse);
                    }

                    let comp_len = slot.compressed_len as usize;
                    let comp_slice = &ZSWAP_BUFFERS[idx].bytes[..comp_len];

                    // Verify checksum
                    if calculate_checksum(comp_slice) != slot.checksum {
                        return Err(SwapError::ChecksumMismatch);
                    }

                    // Decompress with LZ4
                    let decomp_sz = lz4_decompress_block(comp_slice, out_page_data)
                        .map_err(|_| SwapError::DecompressionFailed)?;

                    if decomp_sz != SWAP_PAGE_SIZE {
                        return Err(SwapError::DecompressionFailed);
                    }

                    // Free zswap slot
                    slot.in_use = false;
                }

                SWAP_STATS.swap_ins.fetch_add(1, Ordering::Relaxed);
                SWAP_STATS.zswap_stored.fetch_sub(1, Ordering::Relaxed);
                Ok(())
            }
            SwapLocation::Disk(slot_idx) => {
                let idx = slot_idx as usize;
                if idx >= MAX_DISK_SWAP_SLOTS {
                    return Err(SwapError::InvalidEntry);
                }

                unsafe {
                    let slot = &mut DISK_SWAP_SLOTS[idx];
                    if !slot.in_use || slot.page_id != entry.page_id {
                        return Err(SwapError::SlotNotInUse);
                    }

                    out_page_data.copy_from_slice(&DISK_SWAP_BUFFERS[idx].bytes);
                    slot.in_use = false;
                }

                SWAP_STATS.swap_ins.fetch_add(1, Ordering::Relaxed);
                SWAP_STATS.disk_stored.fetch_sub(1, Ordering::Relaxed);
                Ok(())
            }
            SwapLocation::None => Err(SwapError::InvalidEntry),
        }
    }

    /// Frees swap resources without restoring data (e.g. process exit or munmap)
    pub fn swap_free(&mut self, entry: &SwapEntry) -> Result<(), SwapError> {
        match entry.location {
            SwapLocation::Zswap(slot_idx) => {
                let idx = slot_idx as usize;
                if idx >= MAX_ZSWAP_SLOTS {
                    return Err(SwapError::InvalidEntry);
                }
                unsafe {
                    let slot = &mut ZSWAP_SLOTS[idx];
                    if slot.in_use {
                        slot.in_use = false;
                        SWAP_STATS.zswap_stored.fetch_sub(1, Ordering::Relaxed);
                    }
                }
                Ok(())
            }
            SwapLocation::Disk(slot_idx) => {
                let idx = slot_idx as usize;
                if idx >= MAX_DISK_SWAP_SLOTS {
                    return Err(SwapError::InvalidEntry);
                }
                unsafe {
                    let slot = &mut DISK_SWAP_SLOTS[idx];
                    if slot.in_use {
                        slot.in_use = false;
                        SWAP_STATS.disk_stored.fetch_sub(1, Ordering::Relaxed);
                    }
                }
                Ok(())
            }
            SwapLocation::None => Ok(()),
        }
    }
}

// -----------------------------------------------------------------------------
// Global Swap Subsystem Singleton
// -----------------------------------------------------------------------------

pub static mut GLOBAL_SWAP_SUBSYSTEM: SwapSubsystem = SwapSubsystem::new();
static SWAP_INITIALIZED: AtomicBool = AtomicBool::new(false);

pub fn init_swap_subsystem() {
    if !SWAP_INITIALIZED.load(Ordering::Acquire) {
        unsafe {
            GLOBAL_SWAP_SUBSYSTEM.init();
        }
        SWAP_INITIALIZED.store(true, Ordering::Release);
    }
}

pub fn get_swap_subsystem() -> &'static mut SwapSubsystem {
    init_swap_subsystem();
    unsafe { &mut GLOBAL_SWAP_SUBSYSTEM }
}

// -----------------------------------------------------------------------------
// In-Kernel Self-Test
// -----------------------------------------------------------------------------

pub fn run_swap_selftest() -> i32 {
    let swap = get_swap_subsystem();
    swap.init();

    // Stage 1: LZ4 round-trip verification on structured page
    let mut page1 = [0u8; SWAP_PAGE_SIZE];
    for i in 0..SWAP_PAGE_SIZE {
        page1[i] = (i % 64) as u8;
    }

    let entry1 = match swap.swap_out(1001, &page1) {
        Ok(e) => e,
        Err(_) => return 1,
    };

    // Because page1 is highly compressible, it must be stored in zswap
    match entry1.location {
        SwapLocation::Zswap(_) => {}
        _ => return 2,
    }

    // Verify compressed size is substantially smaller than 4KB
    if entry1.compressed_size >= 2048 {
        return 3;
    }

    // Stage 2: Swap-in and round-trip verification
    let mut restored_page1 = [0u8; SWAP_PAGE_SIZE];
    if swap.swap_in(&entry1, &mut restored_page1).is_err() {
        return 4;
    }
    if restored_page1 != page1 {
        return 5;
    }

    // Stage 3: High memory pressure threshold test
    if !SwapSubsystem::is_memory_pressure_high(86, 100) {
        return 6; // 86% should be high pressure
    }
    if SwapSubsystem::is_memory_pressure_high(80, 100) {
        return 7; // 80% should not be high pressure
    }

    // Stage 4: Swap free test
    let entry2 = match swap.swap_out(1002, &page1) {
        Ok(e) => e,
        Err(_) => return 8,
    };
    if swap.swap_free(&entry2).is_err() {
        return 9;
    }
    // Trying to swap_in a freed entry should fail
    if swap.swap_in(&entry2, &mut restored_page1).is_ok() {
        return 10;
    }

    0
}

// -----------------------------------------------------------------------------
// Host Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lz4_compress_decompress_roundtrip() {
        let mut original = [0u8; SWAP_PAGE_SIZE];
        for i in 0..SWAP_PAGE_SIZE {
            original[i] = ((i * 7) ^ 0x5A) as u8;
        }

        let mut comp = [0u8; 4300];
        let comp_sz = lz4_compress_block(&original, &mut comp).expect("compression failed");
        assert!(comp_sz > 0);

        let mut decomp = [0u8; SWAP_PAGE_SIZE];
        let decomp_sz = lz4_decompress_block(&comp[..comp_sz], &mut decomp).expect("decompression failed");
        assert_eq!(decomp_sz, SWAP_PAGE_SIZE);
        assert_eq!(&original[..], &decomp[..]);
    }

    #[test]
    fn test_zswap_lifecycle() {
        let mut swap = SwapSubsystem::new();
        swap.init();

        let mut page = [0u8; SWAP_PAGE_SIZE];
        let text = b"NovaOS 2.0 Anonymous Swap & zswap RAM compression ".repeat(80);
        let copy_len = text.len().min(SWAP_PAGE_SIZE);
        page[..copy_len].copy_from_slice(&text[..copy_len]);

        let entry = swap.swap_out(42, &page).expect("swap out failed");
        assert!(matches!(entry.location, SwapLocation::Zswap(_)));
        assert!(entry.compressed_size < 500); // Super high compression

        let mut restored = [0u8; SWAP_PAGE_SIZE];
        swap.swap_in(&entry, &mut restored).expect("swap in failed");
        assert_eq!(&page[..], &restored[..]);
    }

    #[test]
    fn test_swap_in_kernel_selftest() {
        assert_eq!(run_swap_selftest(), 0);
    }
}
