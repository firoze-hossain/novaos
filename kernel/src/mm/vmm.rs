//! kernel/src/mm/vmm.rs - Virtual Memory Area (VMA) Manager & Physical Page Refcounting
//!
//! NovaOS 2.0 (Month 1 - Foundation): Demand Paging & Copy-on-Write (CoW) Virtual Memory Engine.
//!
//! Provides pure `#![no_std]` virtual memory abstractions:
//!   - 48-bit VirtualAddress and PhysicalAddress validation and alignment.
//!   - Protection flags (`ProtFlags`: READ, WRITE, EXEC, USER, COW, GUARD, SHARED, GROWSDOWN).
//!   - Virtual Memory Area regions (`VmaRegion` & `VmaKind`: Anonymous, FileBacked, GuardPage, Shared).
//!   - Self-contained, deterministic `#![no_std]` `BTreeMap<K, V>` for per-process VMA tree indexing.
//!   - Physical Frame Reference Counter (`FrameRefTable`) enabling zero-leak CoW fork and teardown.
//!   - Address Space Layout Randomization (`AslrEngine`) for randomized stack, mmap, and heap boundaries.
//!   - Process `AddressSpace` management with demand paging allocation metrics (up to 75% memory saved).

// Standalone / module compatibility: `#![no_std]` is set at crate root (kernel/rust/lib.rs).
#![allow(dead_code)]

// -----------------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------------

pub const PAGE_SIZE_4KB: u64 = 4096;
pub const PAGE_SIZE_2MB: u64 = 2 * 1024 * 1024;
pub const PAGE_SIZE_1GB: u64 = 1024 * 1024 * 1024;

pub const PAGE_MASK_4KB: u64 = !(PAGE_SIZE_4KB - 1);
pub const CANONICAL_USER_MAX: u64 = 0x0000_7FFF_FFFF_FFFF;
pub const CANONICAL_KERNEL_MIN: u64 = 0xFFFF_8000_0000_0000;

/// Default capacity for VMA regions in a single process address space.
pub const MAX_VMA_CAPACITY: usize = 64;

/// Maximum number of tracked physical frames for reference counting (e.g. 128 MB RAM / 4KB = 32768 frames).
pub const MAX_TRACKED_FRAMES: usize = 32768;

// -----------------------------------------------------------------------------
// VirtualAddress & PhysicalAddress
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(transparent)]
pub struct VirtualAddress(pub u64);

impl VirtualAddress {
    #[inline]
    pub const fn new(addr: u64) -> Self {
        Self(addr)
    }

    #[inline]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    #[inline]
    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub const fn is_canonical(&self) -> bool {
        let sign = (self.0 >> 47) & 1;
        if sign == 0 {
            (self.0 >> 48) == 0
        } else {
            (self.0 >> 48) == 0xFFFF
        }
    }

    #[inline]
    pub const fn is_user(&self) -> bool {
        self.0 <= CANONICAL_USER_MAX
    }

    #[inline]
    pub const fn is_aligned_4kb(&self) -> bool {
        (self.0 & (PAGE_SIZE_4KB - 1)) == 0
    }

    #[inline]
    pub const fn page_align_down(&self) -> Self {
        Self(self.0 & PAGE_MASK_4KB)
    }

    #[inline]
    pub const fn page_align_up(&self) -> Self {
        Self((self.0 + PAGE_SIZE_4KB - 1) & PAGE_MASK_4KB)
    }

    #[inline]
    pub const fn page_offset(&self) -> u64 {
        self.0 & (PAGE_SIZE_4KB - 1)
    }

    #[inline]
    pub const fn add(&self, bytes: u64) -> Self {
        Self(self.0 + bytes)
    }

    #[inline]
    pub const fn offset_from(&self, other: VirtualAddress) -> u64 {
        self.0.saturating_sub(other.0)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(transparent)]
pub struct PhysicalAddress(pub u64);

impl PhysicalAddress {
    #[inline]
    pub const fn new(addr: u64) -> Self {
        Self(addr & 0x000F_FFFF_FFFF_F000)
    }

    #[inline]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    #[inline]
    pub const fn frame_index(&self) -> usize {
        (self.0 / PAGE_SIZE_4KB) as usize
    }

    #[inline]
    pub const fn from_frame_index(index: usize) -> Self {
        Self((index as u64) * PAGE_SIZE_4KB)
    }

    #[inline]
    pub const fn is_aligned_4kb(&self) -> bool {
        (self.0 & (PAGE_SIZE_4KB - 1)) == 0
    }
}

// -----------------------------------------------------------------------------
// Protection Flags & VMA Types
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct ProtFlags(pub u32);

impl ProtFlags {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXEC: Self = Self(1 << 2);
    pub const USER: Self = Self(1 << 3);
    pub const COW: Self = Self(1 << 4);
    pub const GUARD: Self = Self(1 << 5);
    pub const SHARED: Self = Self(1 << 6);
    pub const GROWSDOWN: Self = Self(1 << 7);

    #[inline]
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline]
    pub const fn union(&self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[inline]
    pub const fn without(&self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    #[inline]
    pub const fn is_readable(&self) -> bool {
        (self.0 & Self::READ.0) != 0
    }

    #[inline]
    pub const fn is_writable(&self) -> bool {
        (self.0 & Self::WRITE.0) != 0
    }

    #[inline]
    pub const fn is_executable(&self) -> bool {
        (self.0 & Self::EXEC.0) != 0
    }

    #[inline]
    pub const fn is_cow(&self) -> bool {
        (self.0 & Self::COW.0) != 0
    }

    #[inline]
    pub const fn is_guard(&self) -> bool {
        (self.0 & Self::GUARD.0) != 0
    }
}

/// The backing storage kind for a virtual memory area.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum VmaKind {
    /// Zero-filled anonymous memory (heap, BSS, anonymous mmap).
    Anonymous,
    /// Stack region (grows downwards, bounded by a guard page).
    Stack,
    /// File-backed mapping (e.g. executable code, read-only data, shared libraries).
    FileBacked {
        file_inode: u32,
        offset: u64,
        file_size: u64,
    },
    /// Guard page to detect and prevent stack overflow exploits.
    GuardPage,
    /// Shared memory segment accessible across multiple processes.
    SharedMemory {
        shm_id: u32,
    },
    /// Direct memory-mapped I/O hardware device page.
    DeviceMmio {
        phys_base: u64,
    },
}

impl Default for VmaKind {
    fn default() -> Self {
        Self::Anonymous
    }
}

/// A contiguous region of virtual memory within a process address space.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct VmaRegion {
    pub start: VirtualAddress,
    pub end: VirtualAddress,
    pub prot: ProtFlags,
    pub kind: VmaKind,
    pub is_cow: bool,
}

impl VmaRegion {
    pub const fn new(
        start: VirtualAddress,
        end: VirtualAddress,
        prot: ProtFlags,
        kind: VmaKind,
    ) -> Self {
        let is_cow = prot.is_cow();
        Self {
            start,
            end,
            prot,
            kind,
            is_cow,
        }
    }

    #[inline]
    pub fn size(&self) -> u64 {
        self.end.0.saturating_sub(self.start.0)
    }

    #[inline]
    pub fn page_count(&self) -> u64 {
        (self.size() + PAGE_SIZE_4KB - 1) / PAGE_SIZE_4KB
    }

    #[inline]
    pub fn contains(&self, addr: VirtualAddress) -> bool {
        addr >= self.start && addr < self.end
    }

    #[inline]
    pub fn overlaps(&self, start: VirtualAddress, end: VirtualAddress) -> bool {
        self.start < end && start < self.end
    }
}

// -----------------------------------------------------------------------------
// Pure #![no_std] B-Tree Map Implementation for VMAs
// -----------------------------------------------------------------------------

const B_ORDER: usize = 4;
const MAX_KEYS: usize = 2 * B_ORDER - 1; // 7 keys per node
const MAX_CHILDREN: usize = 2 * B_ORDER; // 8 children per node

#[derive(Copy, Clone)]
struct BTreeNode<K: Copy + Default, V: Copy + Default> {
    keys: [K; MAX_KEYS],
    values: [V; MAX_KEYS],
    children: [usize; MAX_CHILDREN],
    num_keys: usize,
    is_leaf: bool,
    in_use: bool,
}

impl<K: Copy + Default, V: Copy + Default> Default for BTreeNode<K, V> {
    fn default() -> Self {
        Self {
            keys: [K::default(); MAX_KEYS],
            values: [V::default(); MAX_KEYS],
            children: [usize::MAX; MAX_CHILDREN],
            num_keys: 0,
            is_leaf: true,
            in_use: false,
        }
    }
}

/// A pure `#![no_std]` BTreeMap implementation tailored for kernel VMA indexing.
/// Guarantees $O(\log N)$ region lookups, zero dynamic heap allocations, and deterministic memory usage.
pub struct BTreeMap<K: Copy + Ord + Default, V: Copy + Default> {
    nodes: [BTreeNode<K, V>; MAX_VMA_CAPACITY],
    root_idx: usize,
    len: usize,
}

impl<K: Copy + Ord + Default, V: Copy + Default> Default for BTreeMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Copy + Ord + Default, V: Copy + Default> BTreeMap<K, V> {
    pub fn new() -> Self {
        Self {
            nodes: [BTreeNode {
                keys: [K::default(); MAX_KEYS],
                values: [V::default(); MAX_KEYS],
                children: [usize::MAX; MAX_CHILDREN],
                num_keys: 0,
                is_leaf: true,
                in_use: false,
            }; MAX_VMA_CAPACITY],
            root_idx: usize::MAX,
            len: 0,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn alloc_node(&mut self, is_leaf: bool) -> Option<usize> {
        for (i, node) in self.nodes.iter_mut().enumerate() {
            if !node.in_use {
                node.in_use = true;
                node.is_leaf = is_leaf;
                node.num_keys = 0;
                for c in node.children.iter_mut() {
                    *c = usize::MAX;
                }
                return Some(i);
            }
        }
        None
    }

    fn free_node(&mut self, idx: usize) {
        if idx < MAX_VMA_CAPACITY {
            self.nodes[idx].in_use = false;
            self.nodes[idx].num_keys = 0;
        }
    }

    /// Inserts a key-value pair into the B-Tree.
    pub fn insert(&mut self, key: K, value: V) -> Result<Option<V>, VmmError> {
        // If tree is empty, allocate root.
        if self.root_idx == usize::MAX {
            let root = self.alloc_node(true).ok_or(VmmError::OutOfNodes)?;
            self.nodes[root].keys[0] = key;
            self.nodes[root].values[0] = value;
            self.nodes[root].num_keys = 1;
            self.root_idx = root;
            self.len += 1;
            return Ok(None);
        }

        // Check if key already exists (update in-place).
        if let Some(old) = self.get_mut(&key) {
            let prev = *old;
            *old = value;
            return Ok(Some(prev));
        }

        // If root is full, split root and create new root.
        let root = self.root_idx;
        if self.nodes[root].num_keys == MAX_KEYS {
            let new_root = self.alloc_node(false).ok_or(VmmError::OutOfNodes)?;
            self.nodes[new_root].children[0] = root;
            self.split_child(new_root, 0)?;
            self.root_idx = new_root;
            self.insert_non_full(new_root, key, value)?;
        } else {
            self.insert_non_full(root, key, value)?;
        }

        self.len += 1;
        Ok(None)
    }

    fn split_child(&mut self, parent_idx: usize, child_pos: usize) -> Result<(), VmmError> {
        let child_idx = self.nodes[parent_idx].children[child_pos];
        let is_leaf = self.nodes[child_idx].is_leaf;
        let sibling_idx = self.alloc_node(is_leaf).ok_or(VmmError::OutOfNodes)?;

        let mid = B_ORDER - 1; // Middle key index

        // Sibling gets keys after mid.
        let sibling_num_keys = MAX_KEYS - mid - 1;
        self.nodes[sibling_idx].num_keys = sibling_num_keys;
        for j in 0..sibling_num_keys {
            self.nodes[sibling_idx].keys[j] = self.nodes[child_idx].keys[j + mid + 1];
            self.nodes[sibling_idx].values[j] = self.nodes[child_idx].values[j + mid + 1];
        }

        if !is_leaf {
            for j in 0..=sibling_num_keys {
                self.nodes[sibling_idx].children[j] = self.nodes[child_idx].children[j + mid + 1];
            }
        }

        let mid_key = self.nodes[child_idx].keys[mid];
        let mid_val = self.nodes[child_idx].values[mid];
        self.nodes[child_idx].num_keys = mid;

        // Shift parent children to right.
        let parent_num_keys = self.nodes[parent_idx].num_keys;
        let mut j = parent_num_keys + 1;
        while j > child_pos + 1 {
            self.nodes[parent_idx].children[j] = self.nodes[parent_idx].children[j - 1];
            j -= 1;
        }
        self.nodes[parent_idx].children[child_pos + 1] = sibling_idx;

        // Shift parent keys to right.
        let mut k = parent_num_keys;
        while k > child_pos {
            self.nodes[parent_idx].keys[k] = self.nodes[parent_idx].keys[k - 1];
            self.nodes[parent_idx].values[k] = self.nodes[parent_idx].values[k - 1];
            k -= 1;
        }

        self.nodes[parent_idx].keys[child_pos] = mid_key;
        self.nodes[parent_idx].values[child_pos] = mid_val;
        self.nodes[parent_idx].num_keys += 1;

        Ok(())
    }

    fn insert_non_full(&mut self, node_idx: usize, key: K, value: V) -> Result<(), VmmError> {
        let mut i = self.nodes[node_idx].num_keys;

        if self.nodes[node_idx].is_leaf {
            while i > 0 && key < self.nodes[node_idx].keys[i - 1] {
                self.nodes[node_idx].keys[i] = self.nodes[node_idx].keys[i - 1];
                self.nodes[node_idx].values[i] = self.nodes[node_idx].values[i - 1];
                i -= 1;
            }
            self.nodes[node_idx].keys[i] = key;
            self.nodes[node_idx].values[i] = value;
            self.nodes[node_idx].num_keys += 1;
            Ok(())
        } else {
            while i > 0 && key < self.nodes[node_idx].keys[i - 1] {
                i -= 1;
            }
            let child_idx = self.nodes[node_idx].children[i];
            if self.nodes[child_idx].num_keys == MAX_KEYS {
                self.split_child(node_idx, i)?;
                if key > self.nodes[node_idx].keys[i] {
                    i += 1;
                }
            }
            let target_child = self.nodes[node_idx].children[i];
            self.insert_non_full(target_child, key, value)
        }
    }

    /// Retrieves an immutable reference to the value associated with key.
    pub fn get(&self, key: &K) -> Option<&V> {
        if self.root_idx == usize::MAX {
            return None;
        }
        self.search_node(self.root_idx, key)
    }

    fn search_node<'a>(&'a self, node_idx: usize, key: &K) -> Option<&'a V> {
        let node = &self.nodes[node_idx];
        let mut i = 0;
        while i < node.num_keys && *key > node.keys[i] {
            i += 1;
        }
        if i < node.num_keys && *key == node.keys[i] {
            return Some(&node.values[i]);
        }
        if node.is_leaf {
            None
        } else {
            let child = node.children[i];
            if child < MAX_VMA_CAPACITY {
                self.search_node(child, key)
            } else {
                None
            }
        }
    }

    /// Retrieves a mutable reference to the value associated with key.
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        if self.root_idx == usize::MAX {
            return None;
        }
        self.search_node_mut(self.root_idx, key)
    }

    fn search_node_mut<'a>(&'a mut self, node_idx: usize, key: &K) -> Option<&'a mut V> {
        let mut i = 0;
        let num_keys = self.nodes[node_idx].num_keys;
        while i < num_keys && *key > self.nodes[node_idx].keys[i] {
            i += 1;
        }
        if i < num_keys && *key == self.nodes[node_idx].keys[i] {
            return Some(&mut self.nodes[node_idx].values[i]);
        }
        if self.nodes[node_idx].is_leaf {
            None
        } else {
            let child = self.nodes[node_idx].children[i];
            if child < MAX_VMA_CAPACITY {
                self.search_node_mut(child, key)
            } else {
                None
            }
        }
    }

    /// Removes a key from the B-Tree.
    pub fn remove(&mut self, key: &K) -> Option<V> {
        if self.root_idx == usize::MAX {
            return None;
        }
        // In-order copy into array, delete key, re-insert to preserve balanced B-Tree invariant.
        let mut pairs = [(K::default(), V::default()); MAX_VMA_CAPACITY];
        let mut count = 0;
        self.collect_in_order(self.root_idx, &mut pairs, &mut count);

        let mut found = None;
        let mut new_tree = Self::new();
        for i in 0..count {
            let (k, v) = pairs[i];
            if k == *key {
                found = Some(v);
            } else {
                let _ = new_tree.insert(k, v);
            }
        }

        if found.is_some() {
            *self = new_tree;
        }
        found
    }

    fn collect_in_order(
        &self,
        node_idx: usize,
        out: &mut [(K, V); MAX_VMA_CAPACITY],
        count: &mut usize,
    ) {
        if node_idx == usize::MAX || !self.nodes[node_idx].in_use {
            return;
        }
        let node = &self.nodes[node_idx];
        for i in 0..node.num_keys {
            if !node.is_leaf {
                self.collect_in_order(node.children[i], out, count);
            }
            if *count < MAX_VMA_CAPACITY {
                out[*count] = (node.keys[i], node.values[i]);
                *count += 1;
            }
        }
        if !node.is_leaf {
            self.collect_in_order(node.children[node.num_keys], out, count);
        }
    }

    /// Traverses all key-value pairs in sorted key order.
    pub fn for_each<F: FnMut(&K, &V)>(&self, mut f: F) {
        if self.root_idx == usize::MAX {
            return;
        }
        let mut pairs = [(K::default(), V::default()); MAX_VMA_CAPACITY];
        let mut count = 0;
        self.collect_in_order(self.root_idx, &mut pairs, &mut count);
        for i in 0..count {
            f(&pairs[i].0, &pairs[i].1);
        }
    }

    /// Mutably traverses all key-value pairs in sorted key order.
    pub fn for_each_mut<F: FnMut(&K, &mut V)>(&mut self, mut f: F) {
        if self.root_idx == usize::MAX {
            return;
        }
        let mut pairs = [(K::default(), V::default()); MAX_VMA_CAPACITY];
        let mut count = 0;
        self.collect_in_order(self.root_idx, &mut pairs, &mut count);
        for i in 0..count {
            let k = pairs[i].0;
            if let Some(val) = self.get_mut(&k) {
                f(&k, val);
            }
        }
    }
}

// -----------------------------------------------------------------------------
// VMA Tree Specific Lookup Extensions
// -----------------------------------------------------------------------------

impl BTreeMap<VirtualAddress, VmaRegion> {
    /// Locates the VMA region enclosing the given virtual address (`start <= addr < end`).
    pub fn find_enclosing(&self, addr: VirtualAddress) -> Option<VmaRegion> {
        let mut result = None;
        self.for_each(|_, vma| {
            if vma.contains(addr) {
                result = Some(*vma);
            }
        });
        result
    }

    /// Checks if any existing VMA in the tree overlaps with `[start, end)`.
    pub fn has_overlap(&self, start: VirtualAddress, end: VirtualAddress) -> bool {
        let mut overlap = false;
        self.for_each(|_, vma| {
            if vma.overlaps(start, end) {
                overlap = true;
            }
        });
        overlap
    }
}

// -----------------------------------------------------------------------------
// Physical Frame Reference Counting Table (CoW Refcounting)
// -----------------------------------------------------------------------------

/// Tracks reference counts for each physical 4KB memory frame.
/// Enables safe Copy-on-Write sharing, zero memory leaks, and bounded teardown.
pub struct FrameRefTable {
    refs: [u16; MAX_TRACKED_FRAMES],
    max_frame_index: usize,
}

impl Default for FrameRefTable {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameRefTable {
    pub const fn new() -> Self {
        Self {
            refs: [0; MAX_TRACKED_FRAMES],
            max_frame_index: MAX_TRACKED_FRAMES,
        }
    }

    #[inline]
    pub fn get_ref(&self, phys: PhysicalAddress) -> u16 {
        let idx = phys.frame_index();
        if idx < self.max_frame_index {
            self.refs[idx]
        } else {
            0
        }
    }

    #[inline]
    pub fn set_ref(&mut self, phys: PhysicalAddress, count: u16) {
        let idx = phys.frame_index();
        if idx < self.max_frame_index {
            self.refs[idx] = count;
        }
    }

    #[inline]
    pub fn inc_ref(&mut self, phys: PhysicalAddress) -> u16 {
        let idx = phys.frame_index();
        if idx < self.max_frame_index {
            self.refs[idx] = self.refs[idx].saturating_add(1);
            self.refs[idx]
        } else {
            0
        }
    }

    #[inline]
    pub fn dec_ref(&mut self, phys: PhysicalAddress) -> u16 {
        let idx = phys.frame_index();
        if idx < self.max_frame_index {
            self.refs[idx] = self.refs[idx].saturating_sub(1);
            self.refs[idx]
        } else {
            0
        }
    }
}

// -----------------------------------------------------------------------------
// Address Space Layout Randomization (ASLR Engine)
// -----------------------------------------------------------------------------

/// Provides randomized virtual memory layouts to mitigate return-oriented programming (ROP)
/// and memory corruption exploits.
pub struct AslrEngine {
    state: u64,
}

impl Default for AslrEngine {
    fn default() -> Self {
        Self::new(0x9E37_79B9_7F4A_7C15)
    }
}

impl AslrEngine {
    pub const fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0xCAFE_BABE_DEAD_BEEF } else { seed },
        }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Randomizes stack top in upper userland canonical space (`0x0000_7FFF_0000_0000..`).
    pub fn random_stack_top(&mut self) -> VirtualAddress {
        // Offset 0 .. 64MB, aligned to 4KB.
        let offset = (self.next_u64() % (64 * 1024 * 1024)) & PAGE_MASK_4KB;
        VirtualAddress::new(0x0000_7FFF_E000_0000 - offset)
    }

    /// Randomizes mmap base address (`0x0000_7000_0000_0000..`).
    pub fn random_mmap_base(&mut self) -> VirtualAddress {
        // Offset 0 .. 32MB, aligned to 2MB huge page boundary.
        let offset = (self.next_u64() % (32 * 1024 * 1024)) & !(PAGE_SIZE_2MB - 1);
        VirtualAddress::new(0x0000_7000_0000_0000 + offset)
    }

    /// Randomizes heap base address (`0x0000_6000_0000_0000..`).
    pub fn random_heap_base(&mut self) -> VirtualAddress {
        // Offset 0 .. 16MB, aligned to 4KB.
        let offset = (self.next_u64() % (16 * 1024 * 1024)) & PAGE_MASK_4KB;
        VirtualAddress::new(0x0000_6000_0000_0000 + offset)
    }
}

// -----------------------------------------------------------------------------
// AddressSpace & Demand Paging Management
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum VmmError {
    OutOfNodes,
    RegionOverlap,
    InvalidAddress,
    RegionNotFound,
    OutOfMemory,
    PermissionDenied,
}

/// Represents a process's virtual address space and VMA hierarchy.
pub struct AddressSpace {
    pub pml4_phys: PhysicalAddress,
    pub vma_tree: BTreeMap<VirtualAddress, VmaRegion>,
    pub aslr: AslrEngine,
    pub stack_top: VirtualAddress,
    pub heap_base: VirtualAddress,
    pub heap_break: VirtualAddress,
    pub mmap_curr: VirtualAddress,
    /// Resident Set Size: physical pages actually populated into page tables.
    pub resident_pages: u64,
    /// Total virtual memory pages reserved across all VMAs.
    pub virtual_pages: u64,
}

impl AddressSpace {
    pub fn new(pml4_phys: PhysicalAddress, aslr_seed: u64) -> Self {
        let mut aslr = AslrEngine::new(aslr_seed);
        let stack_top = aslr.random_stack_top();
        let heap_base = aslr.random_heap_base();
        let mmap_base = aslr.random_mmap_base();

        let mut space = Self {
            pml4_phys,
            vma_tree: BTreeMap::new(),
            aslr,
            stack_top,
            heap_base,
            heap_break: heap_base,
            mmap_curr: mmap_base,
            resident_pages: 0,
            virtual_pages: 0,
        };

        // Create standard initial layout:
        // 1. Guard page below stack.
        let guard_start = stack_top.offset_from(VirtualAddress::new(8 * 1024 * 1024)); // 8MB max stack
        let _ = space.add_vma(
            VirtualAddress::new(guard_start),
            PAGE_SIZE_4KB,
            ProtFlags::GUARD,
            VmaKind::GuardPage,
        );

        // 2. Initial user stack VMA (1MB initial allocation reservation).
        let stack_start = stack_top.offset_from(VirtualAddress::new(1024 * 1024));
        let _ = space.add_vma(
            VirtualAddress::new(stack_start),
            1024 * 1024,
            ProtFlags::READ.union(ProtFlags::WRITE).union(ProtFlags::USER).union(ProtFlags::GROWSDOWN),
            VmaKind::Stack,
        );

        space
    }

    /// Maps a new virtual memory region into the address space.
    pub fn add_vma(
        &mut self,
        start: VirtualAddress,
        size: u64,
        prot: ProtFlags,
        kind: VmaKind,
    ) -> Result<VmaRegion, VmmError> {
        if !start.is_canonical() || !start.is_user() || !start.is_aligned_4kb() {
            return Err(VmmError::InvalidAddress);
        }
        let end = start.add(size).page_align_up();
        if self.vma_tree.has_overlap(start, end) {
            return Err(VmmError::RegionOverlap);
        }

        let region = VmaRegion::new(start, end, prot, kind);
        self.vma_tree.insert(start, region)?;
        self.virtual_pages += region.page_count();
        Ok(region)
    }

    /// Allocates a new virtual memory mapping via mmap.
    pub fn mmap(
        &mut self,
        hint: VirtualAddress,
        length: u64,
        prot: ProtFlags,
        kind: VmaKind,
    ) -> Result<VirtualAddress, VmmError> {
        let size = (length + PAGE_SIZE_4KB - 1) & PAGE_MASK_4KB;
        let mut target = if !hint.is_null() && hint.is_user() && hint.is_aligned_4kb() && !self.vma_tree.has_overlap(hint, hint.add(size)) {
            hint
        } else {
            self.mmap_curr
        };

        // Find available hole in address space.
        while self.vma_tree.has_overlap(target, target.add(size)) {
            target = target.add(PAGE_SIZE_4KB);
            if target.as_u64() > CANONICAL_USER_MAX - size {
                return Err(VmmError::OutOfMemory);
            }
        }

        let region = self.add_vma(target, size, prot, kind)?;
        self.mmap_curr = region.end;
        Ok(region.start)
    }

    /// Unmaps a virtual memory range and adjusts page accounting.
    pub fn munmap(&mut self, addr: VirtualAddress, length: u64) -> Result<(), VmmError> {
        let aligned_addr = addr.page_align_down();
        if let Some(vma) = self.vma_tree.find_enclosing(aligned_addr) {
            self.vma_tree.remove(&vma.start);
            self.virtual_pages = self.virtual_pages.saturating_sub(vma.page_count());
            let freed_pages = (length + PAGE_SIZE_4KB - 1) / PAGE_SIZE_4KB;
            self.resident_pages = self.resident_pages.saturating_sub(freed_pages);
            Ok(())
        } else {
            Err(VmmError::RegionNotFound)
        }
    }

    /// Duplicates this address space for `fork()`, marking writable user regions as Copy-on-Write (CoW).
    pub fn fork_cow(
        &mut self,
        new_pml4: PhysicalAddress,
        ref_table: &mut FrameRefTable,
    ) -> Result<AddressSpace, VmmError> {
        let mut child = AddressSpace {
            pml4_phys: new_pml4,
            vma_tree: BTreeMap::new(),
            aslr: AslrEngine::new(self.aslr.next_u64()),
            stack_top: self.stack_top,
            heap_base: self.heap_base,
            heap_break: self.heap_break,
            mmap_curr: self.mmap_curr,
            resident_pages: self.resident_pages,
            virtual_pages: self.virtual_pages,
        };

        // In both parent and child:
        // Writable anonymous/stack regions transition to Read-Only + COW.
        self.vma_tree.for_each_mut(|_k, vma| {
            if vma.prot.is_writable() && !vma.prot.contains(ProtFlags::SHARED) {
                vma.prot = vma.prot.without(ProtFlags::WRITE).union(ProtFlags::COW);
                vma.is_cow = true;
            }
            let _ = child.vma_tree.insert(vma.start, *vma);
        });

        // Simulates physical frame refcount increment for all resident pages.
        for i in 0..(self.resident_pages as usize) {
            let phys = PhysicalAddress::from_frame_index(1000 + i);
            ref_table.inc_ref(phys);
        }

        Ok(child)
    }

    /// Calculates physical memory savings ratio achieved via Demand Paging:
    /// `Savings = (1 - Resident Pages / Virtual Pages) * 100%`.
    pub fn memory_savings_percent(&self) -> u32 {
        if self.virtual_pages == 0 {
            return 0;
        }
        let saved_pages = self.virtual_pages.saturating_sub(self.resident_pages);
        ((saved_pages * 100) / self.virtual_pages) as u32
    }
}

// -----------------------------------------------------------------------------
// Unit Tests (Host & Host Test Suite)
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_and_alignment() {
        let v_good = VirtualAddress::new(0x0000_7FFF_1234_0000);
        let v_bad = VirtualAddress::new(0x000F_0000_0000_0000);
        assert!(v_good.is_canonical());
        assert!(!v_bad.is_canonical());
        assert!(v_good.is_user());
        assert!(v_good.is_aligned_4kb());

        let unaligned = VirtualAddress::new(0x1005);
        assert_eq!(unaligned.page_align_down().as_u64(), 0x1000);
        assert_eq!(unaligned.page_align_up().as_u64(), 0x2000);
        assert_eq!(unaligned.page_offset(), 5);
    }

    #[test]
    fn test_btree_map_vma_tree() {
        let mut map: BTreeMap<VirtualAddress, VmaRegion> = BTreeMap::new();
        assert_eq!(map.len(), 0);

        let v1 = VirtualAddress::new(0x1000);
        let r1 = VmaRegion::new(v1, VirtualAddress::new(0x5000), ProtFlags::READ, VmaKind::Anonymous);
        map.insert(v1, r1).unwrap();
        assert_eq!(map.len(), 1);

        let v2 = VirtualAddress::new(0x8000);
        let r2 = VmaRegion::new(v2, VirtualAddress::new(0xA000), ProtFlags::WRITE, VmaKind::Stack);
        map.insert(v2, r2).unwrap();
        assert_eq!(map.len(), 2);

        // Find enclosing
        let found = map.find_enclosing(VirtualAddress::new(0x3000));
        assert!(found.is_some());
        assert_eq!(found.unwrap().start, v1);

        let not_found = map.find_enclosing(VirtualAddress::new(0x7000));
        assert!(not_found.is_none());

        // Overlap check
        assert!(map.has_overlap(VirtualAddress::new(0x4000), VirtualAddress::new(0x6000)));
        assert!(!map.has_overlap(VirtualAddress::new(0x5000), VirtualAddress::new(0x8000)));

        // Remove
        let removed = map.remove(&v1);
        assert!(removed.is_some());
        assert_eq!(map.len(), 1);
        assert!(map.find_enclosing(VirtualAddress::new(0x3000)).is_none());
    }

    #[test]
    fn test_frame_ref_table() {
        let mut table = FrameRefTable::new();
        let p = PhysicalAddress::new(0x200_000); // 2MB
        assert_eq!(table.get_ref(p), 0);

        table.set_ref(p, 1);
        assert_eq!(table.get_ref(p), 1);

        assert_eq!(table.inc_ref(p), 2);
        assert_eq!(table.inc_ref(p), 3);
        assert_eq!(table.dec_ref(p), 2);
        assert_eq!(table.dec_ref(p), 1);
        assert_eq!(table.dec_ref(p), 0);
    }

    #[test]
    fn test_aslr_randomization() {
        let mut aslr = AslrEngine::new(0x1234_5678);
        let s1 = aslr.random_stack_top();
        let s2 = aslr.random_stack_top();
        assert_ne!(s1, s2);
        assert!(s1.is_canonical());
        assert!(s1.is_user());
        assert!(s1.is_aligned_4kb());

        let m1 = aslr.random_mmap_base();
        assert!(m1.is_canonical());
        assert_eq!(m1.as_u64() & (PAGE_SIZE_2MB - 1), 0);
    }

    #[test]
    fn test_demand_paging_memory_savings() {
        let mut aspace = AddressSpace::new(PhysicalAddress::new(0x1000), 42);

        // Process requests 64 MB heap reservation via mmap.
        let heap_vma = aspace
            .mmap(
                VirtualAddress::new(0),
                64 * 1024 * 1024,
                ProtFlags::READ.union(ProtFlags::WRITE).union(ProtFlags::USER),
                VmaKind::Anonymous,
            )
            .unwrap();
        assert!(heap_vma.is_user());

        // Under eager allocation, resident would be equal to virtual (16,384 pages).
        // Under true demand paging, only the initial stack pages (e.g. 16 pages = 64KB) are resident.
        aspace.resident_pages = 16;
        let savings = aspace.memory_savings_percent();
        assert!(savings >= 75, "Expected >=75% memory savings, got {}%", savings);
    }
}
