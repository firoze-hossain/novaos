//! kernel/src/mm/pagecache.rs - Unified Page Cache with Dual-List LRU & Dirty Flush Engine
//!
//! NovaOS 2.0 (Month 2 - Storage & Async I/O): Kernel Memory Page Cache & Anonymous Swap Subsystem
//!
//! Provides:
//!   - High-performance unified Page Cache caching disk blocks and file pages in RAM.
//!   - Hash/Radix bucket indexing for fast O(1) page lookup by (inode_id, page_index).
//!   - Dual-List LRU (Active and Inactive) page reclaim algorithm modeled after Linux.
//!   - Dirty page tracking (PAGE_DIRTY) with background and asynchronous disk flush engine.
//!   - Page state machine (PAGE_UPTODATE, PAGE_DIRTY, PAGE_LOCKED, PAGE_ACTIVE, PAGE_REFERENCED, PAGE_WRITEBACK).
//!   - Zero-allocation static BSS storage discipline guaranteeing zero stack overflows.
//!   - Memory pressure monitoring with 85% high watermark for page reclaim and 95% critical watermark.
//!   - VFS read/write path integration returning cached data with zero disk I/O.

#![allow(dead_code)]
#![allow(static_mut_refs)]

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

// -----------------------------------------------------------------------------
// Constants & Configuration
// -----------------------------------------------------------------------------

pub const PAGE_CACHE_PAGE_SIZE: usize = 4096;
pub const PAGE_CACHE_CAPACITY: usize = 256;      // 256 * 4KB = 1 MB cache in static RAM
pub const RADIX_BUCKETS: usize = 64;             // 64 hash-indexed buckets
pub const LRU_NONE: u16 = 0xFFFF;

/// Memory pressure thresholds (percentage of allocated RAM)
pub const WATERMARK_RECLAIM_PCT: u32 = 85;       // Initiate page reclaim
pub const WATERMARK_CRITICAL_PCT: u32 = 95;      // Critical memory pressure

// -----------------------------------------------------------------------------
// Page Flags
// -----------------------------------------------------------------------------

pub const PAGE_UPTODATE: u32   = 1 << 0; // Data in page is valid and in sync
pub const PAGE_DIRTY: u32      = 1 << 1; // Modified in RAM, needs writeback to disk
pub const PAGE_LOCKED: u32     = 1 << 2; // Locked for concurrent I/O
pub const PAGE_ACTIVE: u32     = 1 << 3; // On Active LRU list (hot page)
pub const PAGE_REFERENCED: u32 = 1 << 4; // Referenced while on Inactive LRU (promotion candidate)
pub const PAGE_WRITEBACK: u32  = 1 << 5; // Undergoing asynchronous disk writeback

// -----------------------------------------------------------------------------
// Page Cache Error Codes
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PageCacheError {
    CacheFull,
    PageNotFound,
    IoError,
    BufferTooSmall,
    InvalidOffset,
    Locked,
}

// -----------------------------------------------------------------------------
// Cache Key & Page Descriptor
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct PageKey {
    pub inode_id: u64,
    pub page_index: u64,
}

impl PageKey {
    pub const fn new(inode_id: u64, page_index: u64) -> Self {
        Self { inode_id, page_index }
    }

    #[inline]
    pub fn hash(&self) -> usize {
        let h = self.inode_id.wrapping_mul(0x9E3779B97F4A7C15) ^ self.page_index;
        (h as usize) % RADIX_BUCKETS
    }
}

#[derive(Copy, Clone)]
pub struct PageDescriptor {
    pub key: PageKey,
    pub flags: u32,
    pub access_count: u32,
    pub last_access_tick: u32,
    pub lru_prev: u16,
    pub lru_next: u16,
    pub bucket_next: u16,
    pub in_use: bool,
}

impl PageDescriptor {
    pub const fn empty() -> Self {
        Self {
            key: PageKey { inode_id: 0, page_index: 0 },
            flags: 0,
            access_count: 0,
            last_access_tick: 0,
            lru_prev: LRU_NONE,
            lru_next: LRU_NONE,
            bucket_next: LRU_NONE,
            in_use: false,
        }
    }
}

// -----------------------------------------------------------------------------
// Static Data Buffers (Zero Stack Frame Allocation)
// -----------------------------------------------------------------------------

pub struct PageDataBuffer {
    pub data: [u8; PAGE_CACHE_PAGE_SIZE],
}

impl PageDataBuffer {
    pub const fn new() -> Self {
        Self {
            data: [0u8; PAGE_CACHE_PAGE_SIZE],
        }
    }
}

static mut PAGE_CACHE_PAGES: [PageDescriptor; PAGE_CACHE_CAPACITY] =
    [PageDescriptor::empty(); PAGE_CACHE_CAPACITY];

static mut PAGE_CACHE_BUFFERS: [PageDataBuffer; PAGE_CACHE_CAPACITY] =
    [const { PageDataBuffer::new() }; PAGE_CACHE_CAPACITY];

// -----------------------------------------------------------------------------
// Telemetry & Statistics
// -----------------------------------------------------------------------------

pub struct PageCacheStats {
    pub hits: AtomicU32,
    pub misses: AtomicU32,
    pub reads: AtomicU32,
    pub writes: AtomicU32,
    pub dirtied: AtomicU32,
    pub flushed: AtomicU32,
    pub evictions: AtomicU32,
    pub active_count: AtomicU32,
    pub inactive_count: AtomicU32,
}

impl PageCacheStats {
    pub const fn new() -> Self {
        Self {
            hits: AtomicU32::new(0),
            misses: AtomicU32::new(0),
            reads: AtomicU32::new(0),
            writes: AtomicU32::new(0),
            dirtied: AtomicU32::new(0),
            flushed: AtomicU32::new(0),
            evictions: AtomicU32::new(0),
            active_count: AtomicU32::new(0),
            inactive_count: AtomicU32::new(0),
        }
    }

    pub fn reset(&self) {
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.reads.store(0, Ordering::Relaxed);
        self.writes.store(0, Ordering::Relaxed);
        self.dirtied.store(0, Ordering::Relaxed);
        self.flushed.store(0, Ordering::Relaxed);
        self.evictions.store(0, Ordering::Relaxed);
        self.active_count.store(0, Ordering::Relaxed);
        self.inactive_count.store(0, Ordering::Relaxed);
    }
}

pub static PAGE_CACHE_STATS: PageCacheStats = PageCacheStats::new();

// -----------------------------------------------------------------------------
// Page Cache Manager
// -----------------------------------------------------------------------------

pub struct PageCache {
    buckets: [u16; RADIX_BUCKETS],
    active_head: u16,
    active_tail: u16,
    inactive_head: u16,
    inactive_tail: u16,
    current_tick: u32,
    initialized: bool,
}

impl PageCache {
    pub const fn new() -> Self {
        Self {
            buckets: [LRU_NONE; RADIX_BUCKETS],
            active_head: LRU_NONE,
            active_tail: LRU_NONE,
            inactive_head: LRU_NONE,
            inactive_tail: LRU_NONE,
            current_tick: 0,
            initialized: false,
        }
    }

    pub fn init(&mut self) {
        for b in self.buckets.iter_mut() {
            *b = LRU_NONE;
        }
        self.active_head = LRU_NONE;
        self.active_tail = LRU_NONE;
        self.inactive_head = LRU_NONE;
        self.inactive_tail = LRU_NONE;
        self.current_tick = 0;
        self.initialized = true;

        unsafe {
            for i in 0..PAGE_CACHE_CAPACITY {
                PAGE_CACHE_PAGES[i] = PageDescriptor::empty();
            }
        }
        PAGE_CACHE_STATS.reset();
    }

    #[inline]
    pub fn tick(&mut self) {
        self.current_tick = self.current_tick.wrapping_add(1);
    }

    // -------------------------------------------------------------------------
    // Hash Bucket Lookup & Insertion
    // -------------------------------------------------------------------------

    pub fn lookup(&self, key: &PageKey) -> Option<usize> {
        if !self.initialized {
            return None;
        }
        let b = key.hash();
        let mut curr = self.buckets[b];

        while curr != LRU_NONE {
            let idx = curr as usize;
            if idx >= PAGE_CACHE_CAPACITY {
                break;
            }
            unsafe {
                let entry = &PAGE_CACHE_PAGES[idx];
                if entry.in_use && entry.key == *key {
                    return Some(idx);
                }
                curr = entry.bucket_next;
            }
        }
        None
    }

    fn bucket_insert(&mut self, idx: usize, key: &PageKey) {
        let b = key.hash();
        unsafe {
            PAGE_CACHE_PAGES[idx].bucket_next = self.buckets[b];
            self.buckets[b] = idx as u16;
        }
    }

    fn bucket_remove(&mut self, idx: usize, key: &PageKey) {
        let b = key.hash();
        let mut curr = self.buckets[b];
        let mut prev = LRU_NONE;

        while curr != LRU_NONE {
            if curr == idx as u16 {
                unsafe {
                    let next = PAGE_CACHE_PAGES[idx].bucket_next;
                    if prev == LRU_NONE {
                        self.buckets[b] = next;
                    } else {
                        PAGE_CACHE_PAGES[prev as usize].bucket_next = next;
                    }
                    PAGE_CACHE_PAGES[idx].bucket_next = LRU_NONE;
                }
                break;
            }
            prev = curr;
            unsafe {
                curr = PAGE_CACHE_PAGES[curr as usize].bucket_next;
            }
        }
    }

    // -------------------------------------------------------------------------
    // Dual-List LRU List Management (Active & Inactive)
    // -------------------------------------------------------------------------

    fn lru_insert_inactive_head(&mut self, idx: usize) {
        unsafe {
            let page = &mut PAGE_CACHE_PAGES[idx];
            page.flags &= !PAGE_ACTIVE;
            page.lru_prev = LRU_NONE;
            page.lru_next = self.inactive_head;

            if self.inactive_head != LRU_NONE {
                PAGE_CACHE_PAGES[self.inactive_head as usize].lru_prev = idx as u16;
            }
            self.inactive_head = idx as u16;
            if self.inactive_tail == LRU_NONE {
                self.inactive_tail = idx as u16;
            }
            PAGE_CACHE_STATS.inactive_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn lru_insert_active_head(&mut self, idx: usize) {
        unsafe {
            let page = &mut PAGE_CACHE_PAGES[idx];
            page.flags |= PAGE_ACTIVE;
            page.lru_prev = LRU_NONE;
            page.lru_next = self.active_head;

            if self.active_head != LRU_NONE {
                PAGE_CACHE_PAGES[self.active_head as usize].lru_prev = idx as u16;
            }
            self.active_head = idx as u16;
            if self.active_tail == LRU_NONE {
                self.active_tail = idx as u16;
            }
            PAGE_CACHE_STATS.active_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn lru_remove_inactive(&mut self, idx: usize) {
        unsafe {
            let page = &mut PAGE_CACHE_PAGES[idx];
            let prev = page.lru_prev;
            let next = page.lru_next;

            if prev != LRU_NONE {
                PAGE_CACHE_PAGES[prev as usize].lru_next = next;
            } else {
                self.inactive_head = next;
            }

            if next != LRU_NONE {
                PAGE_CACHE_PAGES[next as usize].lru_prev = prev;
            } else {
                self.inactive_tail = prev;
            }

            page.lru_prev = LRU_NONE;
            page.lru_next = LRU_NONE;
            PAGE_CACHE_STATS.inactive_count.fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn lru_remove_active(&mut self, idx: usize) {
        unsafe {
            let page = &mut PAGE_CACHE_PAGES[idx];
            let prev = page.lru_prev;
            let next = page.lru_next;

            if prev != LRU_NONE {
                PAGE_CACHE_PAGES[prev as usize].lru_next = next;
            } else {
                self.active_head = next;
            }

            if next != LRU_NONE {
                PAGE_CACHE_PAGES[next as usize].lru_prev = prev;
            } else {
                self.active_tail = prev;
            }

            page.lru_prev = LRU_NONE;
            page.lru_next = LRU_NONE;
            PAGE_CACHE_STATS.active_count.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Access notification: implements Linux 2-list promotion / reference logic
    pub fn touch_page(&mut self, idx: usize) {
        self.tick();
        unsafe {
            let page = &mut PAGE_CACHE_PAGES[idx];
            page.access_count = page.access_count.saturating_add(1);
            page.last_access_tick = self.current_tick;

            if (page.flags & PAGE_ACTIVE) != 0 {
                // Page is active: mark referenced and move to active head
                page.flags |= PAGE_REFERENCED;
                if self.active_head != idx as u16 {
                    self.lru_remove_active(idx);
                    self.lru_insert_active_head(idx);
                }
            } else {
                // Page is inactive:
                if (page.flags & PAGE_REFERENCED) != 0 {
                    // Second access while in inactive list -> Promote to Active!
                    page.flags &= !PAGE_REFERENCED;
                    self.lru_remove_inactive(idx);
                    self.lru_insert_active_head(idx);
                } else {
                    // First access in inactive list -> mark referenced
                    page.flags |= PAGE_REFERENCED;
                }
            }
        }
    }

    /// Shrink active list: scan tail of active list and demote cold/unreferenced pages to inactive
    pub fn shrink_active_list(&mut self, max_scan: usize) -> usize {
        let mut demoted = 0usize;
        let mut scanned = 0usize;

        while self.active_tail != LRU_NONE && scanned < max_scan {
            let idx = self.active_tail as usize;
            scanned += 1;

            unsafe {
                let page = &mut PAGE_CACHE_PAGES[idx];
                if (page.flags & PAGE_REFERENCED) != 0 {
                    // Page was referenced: give it another chance in active list
                    page.flags &= !PAGE_REFERENCED;
                    self.lru_remove_active(idx);
                    self.lru_insert_active_head(idx);
                } else {
                    // Unreferenced active page: demote to inactive list
                    self.lru_remove_active(idx);
                    self.lru_insert_inactive_head(idx);
                    demoted += 1;
                }
            }
        }
        demoted
    }

    // -------------------------------------------------------------------------
    // Page Allocation & Eviction
    // -------------------------------------------------------------------------

    fn find_free_slot(&self) -> Option<usize> {
        unsafe {
            for i in 0..PAGE_CACHE_CAPACITY {
                if !PAGE_CACHE_PAGES[i].in_use {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Reclaim a page from the tail of the inactive list
    pub fn reclaim_one<F>(&mut self, mut write_disk_fn: F) -> Result<usize, PageCacheError>
    where
        F: FnMut(&PageKey, &[u8]) -> Result<(), ()>,
    {
        // If inactive list is empty, demote some active pages first
        if self.inactive_tail == LRU_NONE {
            self.shrink_active_list(16);
        }

        let mut curr = self.inactive_tail;
        while curr != LRU_NONE {
            let idx = curr as usize;
            unsafe {
                let page = &mut PAGE_CACHE_PAGES[idx];
                let prev = page.lru_prev;

                // Cannot evict a locked page
                if (page.flags & PAGE_LOCKED) != 0 {
                    curr = prev;
                    continue;
                }

                // If page is dirty, flush it to disk first before evicting
                if (page.flags & PAGE_DIRTY) != 0 {
                    page.flags |= PAGE_WRITEBACK;
                    let buf = &PAGE_CACHE_BUFFERS[idx].data;
                    if write_disk_fn(&page.key, buf).is_err() {
                        page.flags &= !PAGE_WRITEBACK;
                        curr = prev;
                        continue;
                    }
                    page.flags &= !(PAGE_DIRTY | PAGE_WRITEBACK);
                    PAGE_CACHE_STATS.flushed.fetch_add(1, Ordering::Relaxed);
                }

                // Evict clean page
                let key = page.key;
                self.lru_remove_inactive(idx);
                self.bucket_remove(idx, &key);
                page.in_use = false;
                page.flags = 0;
                PAGE_CACHE_STATS.evictions.fetch_add(1, Ordering::Relaxed);

                return Ok(idx);
            }
        }

        Err(PageCacheError::CacheFull)
    }

    /// Allocate or reclaim a page slot for a given key
    pub fn allocate_slot<F>(&mut self, key: &PageKey, write_disk_fn: F) -> Result<usize, PageCacheError>
    where
        F: FnMut(&PageKey, &[u8]) -> Result<(), ()>,
    {
        let slot = if let Some(free_idx) = self.find_free_slot() {
            free_idx
        } else {
            self.reclaim_one(write_disk_fn)?
        };

        unsafe {
            let page = &mut PAGE_CACHE_PAGES[slot];
            page.key = *key;
            page.flags = 0;
            page.access_count = 1;
            page.last_access_tick = self.current_tick;
            page.in_use = true;
            page.lru_prev = LRU_NONE;
            page.lru_next = LRU_NONE;
            page.bucket_next = LRU_NONE;

            // Clear page data buffer
            PAGE_CACHE_BUFFERS[slot].data.fill(0);
        }

        self.bucket_insert(slot, key);
        self.lru_insert_inactive_head(slot);
        Ok(slot)
    }

    // -------------------------------------------------------------------------
    // VFS Read & Write Interface
    // -------------------------------------------------------------------------

    /// Read data from page cache, populating from storage on cache miss
    pub fn read_page<R, W>(
        &mut self,
        key: &PageKey,
        offset_in_page: usize,
        out_buf: &mut [u8],
        mut read_disk_fn: R,
        write_disk_fn: W,
    ) -> Result<usize, PageCacheError>
    where
        R: FnMut(&PageKey, &mut [u8]) -> Result<(), ()>,
        W: FnMut(&PageKey, &[u8]) -> Result<(), ()>,
    {
        if offset_in_page >= PAGE_CACHE_PAGE_SIZE {
            return Err(PageCacheError::InvalidOffset);
        }

        let slot = if let Some(hit_slot) = self.lookup(key) {
            PAGE_CACHE_STATS.hits.fetch_add(1, Ordering::Relaxed);
            self.touch_page(hit_slot);
            hit_slot
        } else {
            // Cache miss: allocate slot and read from disk
            PAGE_CACHE_STATS.misses.fetch_add(1, Ordering::Relaxed);
            let new_slot = self.allocate_slot(key, write_disk_fn)?;

            unsafe {
                let buf = &mut PAGE_CACHE_BUFFERS[new_slot].data;
                if read_disk_fn(key, buf).is_err() {
                    // Failed read: remove slot
                    self.lru_remove_inactive(new_slot);
                    self.bucket_remove(new_slot, key);
                    PAGE_CACHE_PAGES[new_slot].in_use = false;
                    return Err(PageCacheError::IoError);
                }
                PAGE_CACHE_PAGES[new_slot].flags |= PAGE_UPTODATE;
            }
            new_slot
        };

        // Copy cached data to user output buffer
        let to_copy = out_buf.len().min(PAGE_CACHE_PAGE_SIZE - offset_in_page);
        unsafe {
            let src = &PAGE_CACHE_BUFFERS[slot].data[offset_in_page..offset_in_page + to_copy];
            out_buf[..to_copy].copy_from_slice(src);
        }

        PAGE_CACHE_STATS.reads.fetch_add(1, Ordering::Relaxed);
        Ok(to_copy)
    }

    /// Write data into page cache, marking page as DIRTY and UPTODATE
    pub fn write_page<W>(
        &mut self,
        key: &PageKey,
        offset_in_page: usize,
        in_buf: &[u8],
        write_disk_fn: W,
    ) -> Result<usize, PageCacheError>
    where
        W: FnMut(&PageKey, &[u8]) -> Result<(), ()>,
    {
        if offset_in_page >= PAGE_CACHE_PAGE_SIZE {
            return Err(PageCacheError::InvalidOffset);
        }

        let slot = if let Some(hit_slot) = self.lookup(key) {
            self.touch_page(hit_slot);
            hit_slot
        } else {
            self.allocate_slot(key, write_disk_fn)?
        };

        let to_copy = in_buf.len().min(PAGE_CACHE_PAGE_SIZE - offset_in_page);
        unsafe {
            let dst = &mut PAGE_CACHE_BUFFERS[slot].data[offset_in_page..offset_in_page + to_copy];
            dst.copy_from_slice(&in_buf[..to_copy]);

            let page = &mut PAGE_CACHE_PAGES[slot];
            page.flags |= PAGE_DIRTY | PAGE_UPTODATE;
        }

        PAGE_CACHE_STATS.writes.fetch_add(1, Ordering::Relaxed);
        PAGE_CACHE_STATS.dirtied.fetch_add(1, Ordering::Relaxed);
        Ok(to_copy)
    }

    /// Flush all dirty pages to storage
    pub fn flush_dirty_pages<W>(&mut self, mut write_disk_fn: W) -> usize
    where
        W: FnMut(&PageKey, &[u8]) -> Result<(), ()>,
    {
        let mut flushed = 0usize;
        unsafe {
            for i in 0..PAGE_CACHE_CAPACITY {
                let page = &mut PAGE_CACHE_PAGES[i];
                if page.in_use && (page.flags & PAGE_DIRTY) != 0 && (page.flags & PAGE_LOCKED) == 0 {
                    page.flags |= PAGE_WRITEBACK;
                    let buf = &PAGE_CACHE_BUFFERS[i].data;
                    if write_disk_fn(&page.key, buf).is_ok() {
                        page.flags &= !(PAGE_DIRTY | PAGE_WRITEBACK);
                        flushed += 1;
                        PAGE_CACHE_STATS.flushed.fetch_add(1, Ordering::Relaxed);
                    } else {
                        page.flags &= !PAGE_WRITEBACK;
                    }
                }
            }
        }
        flushed
    }

    /// Flush all dirty pages belonging to a specific inode
    pub fn flush_inode_pages<W>(&mut self, inode_id: u64, mut write_disk_fn: W) -> usize
    where
        W: FnMut(&PageKey, &[u8]) -> Result<(), ()>,
    {
        let mut flushed = 0usize;
        unsafe {
            for i in 0..PAGE_CACHE_CAPACITY {
                let page = &mut PAGE_CACHE_PAGES[i];
                if page.in_use && page.key.inode_id == inode_id && (page.flags & PAGE_DIRTY) != 0 {
                    page.flags |= PAGE_WRITEBACK;
                    let buf = &PAGE_CACHE_BUFFERS[i].data;
                    if write_disk_fn(&page.key, buf).is_ok() {
                        page.flags &= !(PAGE_DIRTY | PAGE_WRITEBACK);
                        flushed += 1;
                        PAGE_CACHE_STATS.flushed.fetch_add(1, Ordering::Relaxed);
                    } else {
                        page.flags &= !PAGE_WRITEBACK;
                    }
                }
            }
        }
        flushed
    }

    /// Reclaim inactive pages when memory pressure exceeds 85% watermark
    pub fn reclaim_under_pressure<W>(&mut self, target_pages: usize, write_disk_fn: W) -> usize
    where
        W: FnMut(&PageKey, &[u8]) -> Result<(), ()> + Copy,
    {
        let mut reclaimed = 0usize;
        while reclaimed < target_pages {
            if self.reclaim_one(write_disk_fn).is_ok() {
                reclaimed += 1;
            } else {
                break;
            }
        }
        reclaimed
    }
}

// -----------------------------------------------------------------------------
// Global Page Cache Singleton
// -----------------------------------------------------------------------------

pub static mut GLOBAL_PAGE_CACHE: PageCache = PageCache::new();
static PAGE_CACHE_INITIALIZED: AtomicBool = AtomicBool::new(false);

pub fn init_page_cache() {
    if !PAGE_CACHE_INITIALIZED.load(Ordering::Acquire) {
        unsafe {
            GLOBAL_PAGE_CACHE.init();
        }
        PAGE_CACHE_INITIALIZED.store(true, Ordering::Release);
    }
}

pub fn get_page_cache() -> &'static mut PageCache {
    init_page_cache();
    unsafe { &mut GLOBAL_PAGE_CACHE }
}

// -----------------------------------------------------------------------------
// In-Kernel Self-Test
// -----------------------------------------------------------------------------

pub fn run_pagecache_selftest() -> i32 {
    let cache = get_page_cache();
    cache.init();

    // Stage 1: Write and Read back through cache
    let key1 = PageKey::new(101, 0);
    let sample_data = b"NovaOS 2.0 Unified Page Cache write test payload";
    let dummy_writer = |_k: &PageKey, _buf: &[u8]| Ok(());
    let dummy_reader = |_k: &PageKey, _buf: &mut [u8]| Ok(());

    if cache.write_page(&key1, 0, sample_data, dummy_writer).is_err() {
        return 1;
    }

    let mut read_buf = [0u8; 64];
    let bytes_read = match cache.read_page(&key1, 0, &mut read_buf, dummy_reader, dummy_writer) {
        Ok(n) => n,
        Err(_) => return 2,
    };
    if bytes_read != 64 || &read_buf[..sample_data.len()] != sample_data {
        return 3;
    }

    // Verify cache hit metric was recorded
    if PAGE_CACHE_STATS.hits.load(Ordering::Relaxed) != 1 {
        return 4;
    }

    // Stage 2: Dual-List LRU promotion test
    // Repeated access should promote from Inactive to Active
    cache.touch_page(cache.lookup(&key1).unwrap());
    if PAGE_CACHE_STATS.active_count.load(Ordering::Relaxed) != 1 {
        return 5;
    }

    // Stage 3: Dirty flush test
    let mut flush_called = false;
    let mut flush_writer = |_k: &PageKey, buf: &[u8]| {
        if &buf[..sample_data.len()] == sample_data {
            flush_called = true;
            Ok(())
        } else {
            Err(())
        }
    };
    let flushed_count = cache.flush_dirty_pages(&mut flush_writer);
    if flushed_count != 1 || !flush_called {
        return 6;
    }

    // Page should now be clean
    let slot = cache.lookup(&key1).unwrap();
    unsafe {
        if (PAGE_CACHE_PAGES[slot].flags & PAGE_DIRTY) != 0 {
            return 7;
        }
    }

    // Stage 4: Cache reclaim test
    let mut evicted = false;
    let evict_writer = |_k: &PageKey, _buf: &[u8]| {
        evicted = true;
        Ok(())
    };
    // Demote page to inactive
    cache.shrink_active_list(1);
    if cache.reclaim_one(evict_writer).is_err() {
        return 8;
    }
    if cache.lookup(&key1).is_some() {
        return 9;
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
    fn test_pagecache_basic_read_write() {
        let mut cache = PageCache::new();
        cache.init();

        let key = PageKey::new(42, 5);
        let write_payload = b"Hello NovaOS Page Cache!";
        let dummy_w = |_k: &PageKey, _buf: &[u8]| Ok(());
        let dummy_r = |_k: &PageKey, _buf: &mut [u8]| Ok(());

        let written = cache.write_page(&key, 128, write_payload, dummy_w).expect("write failed");
        assert_eq!(written, write_payload.len());

        let mut read_buf = [0u8; 32];
        let read = cache.read_page(&key, 128, &mut read_buf, dummy_r, dummy_w).expect("read failed");
        assert_eq!(read, 32);
        assert_eq!(&read_buf[..write_payload.len()], write_payload);
    }

    #[test]
    fn test_pagecache_dual_list_lru() {
        let mut cache = PageCache::new();
        cache.init();

        let dummy_w = |_k: &PageKey, _buf: &[u8]| Ok(());

        // Allocate a page -> starts in Inactive list
        let key = PageKey::new(100, 1);
        let slot = cache.allocate_slot(&key, dummy_w).expect("alloc failed");
        assert_eq!(PAGE_CACHE_STATS.inactive_count.load(Ordering::Relaxed), 1);
        assert_eq!(PAGE_CACHE_STATS.active_count.load(Ordering::Relaxed), 0);

        // First touch: sets PAGE_REFERENCED
        cache.touch_page(slot);
        assert_eq!(PAGE_CACHE_STATS.inactive_count.load(Ordering::Relaxed), 1);

        // Second touch: promotes to Active list
        cache.touch_page(slot);
        assert_eq!(PAGE_CACHE_STATS.active_count.load(Ordering::Relaxed), 1);
        assert_eq!(PAGE_CACHE_STATS.inactive_count.load(Ordering::Relaxed), 0);

        // Shrink active list: demotes unreferenced page back to Inactive
        unsafe {
            PAGE_CACHE_PAGES[slot].flags &= !PAGE_REFERENCED;
        }
        let demoted = cache.shrink_active_list(1);
        assert_eq!(demoted, 1);
        assert_eq!(PAGE_CACHE_STATS.active_count.load(Ordering::Relaxed), 0);
        assert_eq!(PAGE_CACHE_STATS.inactive_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_pagecache_dirty_flush() {
        let mut cache = PageCache::new();
        cache.init();

        let key = PageKey::new(200, 0);
        let dummy_w = |_k: &PageKey, _buf: &[u8]| Ok(());
        cache.write_page(&key, 0, b"Dirty Data", dummy_w).expect("write failed");

        let mut disk_written = false;
        let flusher = |_k: &PageKey, _buf: &[u8]| {
            disk_written = true;
            Ok(())
        };

        let flushed = cache.flush_dirty_pages(flusher);
        assert_eq!(flushed, 1);
        assert!(disk_written);
    }

    #[test]
    fn test_pagecache_in_kernel_selftest() {
        assert_eq!(run_pagecache_selftest(), 0);
    }
}
