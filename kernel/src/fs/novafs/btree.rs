//! kernel/src/fs/novafs/btree.rs - NovaFS B-Tree Indexing, 64-Bit Extents & SHA-256 Integrity
//!
//! NovaOS 2.0 (Month 2 - Storage & Async I/O): Enterprise Copy-on-Write Filesystem.
//!
//! Features:
//!   - Modeled after Btrfs & ZFS transactional B-tree architectures.
//!   - 64-bit on-disk composite keys: `(object_id, item_type, offset)`.
//!   - Inode metadata, hierarchical directory entries, and 64-bit extents.
//!   - Copy-on-Write (CoW) node duplication ensuring immutable historical states.
//!   - Pure `#![no_std]` SHA-256 block checksumming for automatic bit-rot detection.

#![allow(dead_code, unused_variables)]

// ============================================================================
// NovaFS Error Codes
// ============================================================================

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NovaFsError {
    IoError,
    BlockOutOfRange,
    InvalidMagic,
    InvalidSuperblock,
    ChecksumMismatch,    // Bit-rot or corrupted block detected!
    BTreeFull,
    KeyNotFound,
    SubvolumeNotFound,
    SubvolumeReadOnly,
    JournalFull,
    OutOfSpace,
    InvalidPath,
    CorruptedNode,
}

// ============================================================================
// Constants & Magic Numbers
// ============================================================================

pub const BTREE_NODE_MAGIC: u32 = 0x5452424E; // "NBRT" in little-endian
pub const BTREE_MAX_ITEMS: usize = 32;
pub const BLOCK_SIZE: usize = 4096;

pub const ITEM_TYPE_INODE: u8       = 1;
pub const ITEM_TYPE_DIR_ENTRY: u8   = 2;
pub const ITEM_TYPE_EXTENT_DATA: u8 = 3;
pub const ITEM_TYPE_SUBVOL: u8      = 4;

pub const EXTENT_FLAG_COW: u32        = 1 << 0;
pub const EXTENT_FLAG_COMPRESSED: u32 = 1 << 1;
pub const EXTENT_FLAG_PREALLOC: u32   = 1 << 2;

// ============================================================================
// B-Tree Key Structure
// ============================================================================

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub struct BTreeKey {
    pub object_id: u64, // Inode number or Subvolume ID
    pub item_type: u8,  // ITEM_TYPE_*
    pub offset: u64,    // File logical byte offset or directory name hash
}

impl BTreeKey {
    pub const fn new(object_id: u64, item_type: u8, offset: u64) -> Self {
        Self { object_id, item_type, offset }
    }

    pub const fn inode(ino: u64) -> Self {
        Self::new(ino, ITEM_TYPE_INODE, 0)
    }

    pub const fn dir_entry(parent_ino: u64, name_hash: u64) -> Self {
        Self::new(parent_ino, ITEM_TYPE_DIR_ENTRY, name_hash)
    }

    pub const fn extent(ino: u64, offset: u64) -> Self {
        Self::new(ino, ITEM_TYPE_EXTENT_DATA, offset)
    }

    pub const fn subvolume(subvol_id: u64) -> Self {
        Self::new(subvol_id, ITEM_TYPE_SUBVOL, 0)
    }
}

// ============================================================================
// Item Payloads
// ============================================================================

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct InodeItem {
    pub size: u64,          // File size in bytes
    pub num_blocks: u64,    // Number of 4096-byte blocks allocated
    pub nlink: u32,         // Hard link reference count
    pub uid: u32,           // Owner user ID
    pub gid: u32,           // Group ID
    pub mode: u32,          // POSIX file mode (S_IFREG, S_IFDIR, permissions)
    pub flags: u32,         // Inode flags (immutable, CoW, etc.)
    pub atime: u64,         // Access timestamp
    pub mtime: u64,         // Modification timestamp
    pub ctime: u64,         // Change timestamp
}

impl InodeItem {
    pub const fn new(mode: u32, uid: u32, gid: u32) -> Self {
        Self {
            size: 0,
            num_blocks: 0,
            nlink: 1,
            uid,
            gid,
            mode,
            flags: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DirEntryItem {
    pub child_ino: u64,     // Inode number of entry
    pub file_type: u8,      // 1=Regular, 2=Directory, 3=Symlink
    pub name_len: u8,       // Length of name string
    pub name: [u8; 54],     // Filename buffer
}

impl DirEntryItem {
    pub fn new(child_ino: u64, file_type: u8, name_str: &str) -> Self {
        let bytes = name_str.as_bytes();
        let len = if bytes.len() > 54 { 54 } else { bytes.len() };
        let mut name = [0u8; 54];
        let mut i = 0;
        while i < len {
            name[i] = bytes[i];
            i += 1;
        }
        Self {
            child_ino,
            file_type,
            name_len: len as u8,
            name,
        }
    }

    pub fn name_as_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("")
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ExtentDataItem {
    pub logical_offset: u64,    // File byte offset
    pub physical_block: u64,    // 64-bit disk block number
    pub num_blocks: u32,        // Span in blocks
    pub byte_len: u32,          // Actual payload length
    pub checksum: [u8; 32],     // SHA-256 data block checksum
    pub flags: u32,             // EXTENT_FLAG_*
}

impl ExtentDataItem {
    pub fn new(logical_offset: u64, physical_block: u64, data: &[u8]) -> Self {
        let csum = sha256(data);
        let blocks = ((data.len() + BLOCK_SIZE - 1) / BLOCK_SIZE) as u32;
        Self {
            logical_offset,
            physical_block,
            num_blocks: if blocks == 0 { 1 } else { blocks },
            byte_len: data.len() as u32,
            checksum: csum,
            flags: EXTENT_FLAG_COW,
        }
    }

    /// Validates data against the stored SHA-256 checksum to detect bit-rot.
    pub fn verify_integrity(&self, data: &[u8]) -> bool {
        let len = self.byte_len as usize;
        let slice = if data.len() >= len { &data[..len] } else { data };
        let csum = sha256(slice);
        csum == self.checksum
    }
}

// ============================================================================
// B-Tree Item & Node Header
// ============================================================================

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct BTreeItem {
    pub key: BTreeKey,
    pub child_block: u64,    // For internal nodes (level > 0): points to child block
    pub payload_len: u16,    // Length of active payload bytes
    pub payload: [u8; 64],   // Storage for InodeItem, DirEntryItem, ExtentDataItem
}

impl BTreeItem {
    pub const fn empty() -> Self {
        Self {
            key: BTreeKey::new(0, 0, 0),
            child_block: 0,
            payload_len: 0,
            payload: [0u8; 64],
        }
    }

    pub fn from_inode(ino: u64, inode: &InodeItem) -> Self {
        let mut item = Self::empty();
        item.key = BTreeKey::inode(ino);
        let src = unsafe {
            core::slice::from_raw_parts(
                inode as *const InodeItem as *const u8,
                core::mem::size_of::<InodeItem>(),
            )
        };
        let len = src.len().min(64);
        item.payload[..len].copy_from_slice(&src[..len]);
        item.payload_len = len as u16;
        item
    }

    pub fn as_inode(&self) -> Option<InodeItem> {
        if self.key.item_type != ITEM_TYPE_INODE {
            return None;
        }
        let size = core::mem::size_of::<InodeItem>();
        if (self.payload_len as usize) < size {
            return None;
        }
        unsafe { Some(core::ptr::read_unaligned(self.payload.as_ptr() as *const InodeItem)) }
    }

    pub fn from_dir_entry(parent_ino: u64, name_hash: u64, dir: &DirEntryItem) -> Self {
        let mut item = Self::empty();
        item.key = BTreeKey::dir_entry(parent_ino, name_hash);
        let src = unsafe {
            core::slice::from_raw_parts(
                dir as *const DirEntryItem as *const u8,
                core::mem::size_of::<DirEntryItem>(),
            )
        };
        let len = src.len().min(64);
        item.payload[..len].copy_from_slice(&src[..len]);
        item.payload_len = len as u16;
        item
    }

    pub fn as_dir_entry(&self) -> Option<DirEntryItem> {
        if self.key.item_type != ITEM_TYPE_DIR_ENTRY {
            return None;
        }
        let size = core::mem::size_of::<DirEntryItem>();
        if (self.payload_len as usize) < size {
            return None;
        }
        unsafe { Some(core::ptr::read_unaligned(self.payload.as_ptr() as *const DirEntryItem)) }
    }

    pub fn from_extent(ino: u64, offset: u64, extent: &ExtentDataItem) -> Self {
        let mut item = Self::empty();
        item.key = BTreeKey::extent(ino, offset);
        let src = unsafe {
            core::slice::from_raw_parts(
                extent as *const ExtentDataItem as *const u8,
                core::mem::size_of::<ExtentDataItem>(),
            )
        };
        let len = src.len().min(64);
        item.payload[..len].copy_from_slice(&src[..len]);
        item.payload_len = len as u16;
        item
    }

    pub fn as_extent(&self) -> Option<ExtentDataItem> {
        if self.key.item_type != ITEM_TYPE_EXTENT_DATA {
            return None;
        }
        let size = core::mem::size_of::<ExtentDataItem>();
        if (self.payload_len as usize) < size {
            return None;
        }
        unsafe { Some(core::ptr::read_unaligned(self.payload.as_ptr() as *const ExtentDataItem)) }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct BTreeNodeHeader {
    pub magic: u32,
    pub level: u8,               // 0 = leaf node, >0 = internal index node
    pub flags: u8,
    pub num_items: u16,
    pub generation: u64,         // Monotonic transaction generation
    pub block_num: u64,          // Physical block address on disk
    pub checksum: [u8; 32],      // SHA-256 of header + items
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct BTreeNode {
    pub header: BTreeNodeHeader,
    pub items: [BTreeItem; BTREE_MAX_ITEMS],
    pub padding: [u8; 120],      // Pads struct close to 4096 bytes if needed
}

impl BTreeNode {
    pub const fn empty() -> Self {
        Self {
            header: BTreeNodeHeader {
                magic: BTREE_NODE_MAGIC,
                level: 0,
                flags: 0,
                num_items: 0,
                generation: 1,
                block_num: 0,
                checksum: [0u8; 32],
            },
            items: [BTreeItem::empty(); BTREE_MAX_ITEMS],
            padding: [0u8; 120],
        }
    }

    pub fn new(block_num: u64, generation: u64, level: u8) -> Self {
        let mut node = Self::empty();
        node.header.block_num = block_num;
        node.header.generation = generation;
        node.header.level = level;
        node.recompute_checksum();
        node
    }

    /// Recomputes the SHA-256 checksum over the entire node contents.
    pub fn recompute_checksum(&mut self) {
        self.header.checksum = [0u8; 32];
        let bytes = unsafe {
            core::slice::from_raw_parts(
                self as *const BTreeNode as *const u8,
                core::mem::size_of::<BTreeNode>(),
            )
        };
        self.header.checksum = sha256(bytes);
    }

    /// Verifies node integrity using its stored SHA-256 checksum.
    pub fn verify_checksum(&self) -> bool {
        let mut copy = *self;
        copy.header.checksum = [0u8; 32];
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &copy as *const BTreeNode as *const u8,
                core::mem::size_of::<BTreeNode>(),
            )
        };
        sha256(bytes) == self.header.checksum
    }

    /// Binary search for key or insertion index.
    pub fn search(&self, key: &BTreeKey) -> Result<usize, usize> {
        let mut low = 0;
        let mut high = self.header.num_items as usize;

        while low < high {
            let mid = low + (high - low) / 2;
            let mid_key = &self.items[mid].key;
            if mid_key == key {
                return Ok(mid);
            } else if mid_key < key {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        Err(low)
    }

    /// Inserts or replaces an item in sorted key order.
    pub fn insert(&mut self, item: BTreeItem) -> Result<(), NovaFsError> {
        match self.search(&item.key) {
            Ok(idx) => {
                // Key exists: update in place
                self.items[idx] = item;
                self.recompute_checksum();
                Ok(())
            }
            Err(idx) => {
                // Key not found: insert at index
                let count = self.header.num_items as usize;
                if count >= BTREE_MAX_ITEMS {
                    return Err(NovaFsError::BTreeFull);
                }
                let mut i = count;
                while i > idx {
                    self.items[i] = self.items[i - 1];
                    i -= 1;
                }
                self.items[idx] = item;
                self.header.num_items += 1;
                self.recompute_checksum();
                Ok(())
            }
        }
    }

    /// Look up an item by key.
    pub fn get(&self, key: &BTreeKey) -> Option<&BTreeItem> {
        match self.search(key) {
            Ok(idx) => Some(&self.items[idx]),
            Err(_) => None,
        }
    }

    /// Look up a mutable item by key.
    pub fn get_mut(&mut self, key: &BTreeKey) -> Option<&mut BTreeItem> {
        match self.search(key) {
            Ok(idx) => Some(&mut self.items[idx]),
            Err(_) => None,
        }
    }

    /// Removes an item by key.
    pub fn remove(&mut self, key: &BTreeKey) -> Result<(), NovaFsError> {
        match self.search(key) {
            Ok(idx) => {
                let count = self.header.num_items as usize;
                let mut i = idx;
                while i + 1 < count {
                    self.items[i] = self.items[i + 1];
                    i += 1;
                }
                self.items[count - 1] = BTreeItem::empty();
                self.header.num_items -= 1;
                self.recompute_checksum();
                Ok(())
            }
            Err(_) => Err(NovaFsError::KeyNotFound),
        }
    }

    /// Creates a Copy-on-Write duplicate of this node for a new physical block and generation.
    pub fn cow_duplicate(&self, new_block_num: u64, new_generation: u64) -> Self {
        let mut dup = *self;
        dup.header.block_num = new_block_num;
        dup.header.generation = new_generation;
        dup.recompute_checksum();
        dup
    }
}

// ============================================================================
// Pure #[no_std] SHA-256 Engine (FIPS 180-4)
// ============================================================================

const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
    0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
    0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
    0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[inline(always)]
fn rotr(x: u32, n: u32) -> u32 {
    (x >> n) | (x << (32 - n))
}

fn process_sha256_block(state: &mut [u32; 8], block: &[u8]) {
    let mut w = [0u32; 64];
    let mut i = 0;
    while i < 16 {
        w[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
        i += 1;
    }
    while i < 64 {
        let s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
        let s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        i += 1;
    }

    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];
    let mut e = state[4];
    let mut f = state[5];
    let mut g = state[6];
    let mut h = state[7];

    let mut t = 0;
    while t < 64 {
        let s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[t]).wrapping_add(w[t]);
        let s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
        t += 1;
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

/// Compute SHA-256 for arbitrary byte slices in pure `#![no_std]`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut state = H0;
    let mut offset = 0;
    let total_bytes = data.len();

    // Process all complete 64-byte blocks
    while offset + 64 <= total_bytes {
        process_sha256_block(&mut state, &data[offset..offset + 64]);
        offset += 64;
    }

    // Prepare final padded block(s)
    let rem = total_bytes - offset;
    let mut buf = [0u8; 128];
    if rem > 0 {
        buf[..rem].copy_from_slice(&data[offset..]);
    }
    buf[rem] = 0x80;

    let pad_len = if rem < 56 { 64 } else { 128 };
    let bit_len = (total_bytes as u64) * 8;
    buf[pad_len - 8..pad_len].copy_from_slice(&bit_len.to_be_bytes());

    process_sha256_block(&mut state, &buf[..64]);
    if pad_len == 128 {
        process_sha256_block(&mut state, &buf[64..128]);
    }

    let mut out = [0u8; 32];
    let mut j = 0;
    while j < 8 {
        out[j * 4..j * 4 + 4].copy_from_slice(&state[j].to_be_bytes());
        j += 1;
    }
    out
}

// ============================================================================
// FNV-1a Hash for Directory Name Keys
// ============================================================================

pub fn fnv1a_name_hash(name: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &byte in name.as_bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3u64);
    }
    hash
}

// ============================================================================
// Unit Tests (Host-side cargo test / rustc --test)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_known_vectors() {
        // Standard FIPS 180-4 test vectors
        let empty_hash = sha256(b"");
        assert_eq!(
            empty_hash,
            [
                0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14,
                0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9, 0x24,
                0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c,
                0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52, 0xb8, 0x55,
            ]
        );

        let abc_hash = sha256(b"abc");
        assert_eq!(
            abc_hash,
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea,
                0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
                0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c,
                0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
            ]
        );
    }

    #[test]
    fn test_btree_key_ordering() {
        let k1 = BTreeKey::new(1, ITEM_TYPE_INODE, 0);
        let k2 = BTreeKey::new(1, ITEM_TYPE_DIR_ENTRY, 100);
        let k3 = BTreeKey::new(2, ITEM_TYPE_INODE, 0);

        assert!(k1 < k2);
        assert!(k2 < k3);
        assert_eq!(k1, BTreeKey::inode(1));
    }

    #[test]
    fn test_btree_node_insert_lookup() {
        let mut node = BTreeNode::new(10, 1, 0);
        assert!(node.verify_checksum());

        let inode = InodeItem::new(0o100644, 1000, 1000);
        let item1 = BTreeItem::from_inode(5, &inode);
        node.insert(item1).expect("insert item1 failed");

        let dir = DirEntryItem::new(12, 1, "config.sys");
        let hash = fnv1a_name_hash("config.sys");
        let item2 = BTreeItem::from_dir_entry(5, hash, &dir);
        node.insert(item2).expect("insert item2 failed");

        assert_eq!(node.header.num_items, 2);
        assert!(node.verify_checksum());

        let looked_up = node.get(&BTreeKey::inode(5)).expect("inode not found");
        let recovered_inode = looked_up.as_inode().expect("payload decode failed");
        assert_eq!(recovered_inode.mode, 0o100644);

        let looked_up_dir = node.get(&BTreeKey::dir_entry(5, hash)).expect("dir not found");
        let recovered_dir = looked_up_dir.as_dir_entry().expect("payload decode failed");
        assert_eq!(recovered_dir.name_as_str(), "config.sys");
        assert_eq!(recovered_dir.child_ino, 12);
    }

    #[test]
    fn test_extent_checksum_and_bitrot() {
        let mut data = [0u8; 4096];
        data[0] = 0xAA;
        data[4095] = 0x55;

        let extent = ExtentDataItem::new(0, 100, &data);
        assert!(extent.verify_integrity(&data));

        // Inject bit-rot: flip a single bit in the data
        data[2048] ^= 0x01;
        assert!(!extent.verify_integrity(&data), "Bit-rot must be caught by SHA-256!");
    }

    #[test]
    fn test_cow_node_duplication() {
        let mut node1 = BTreeNode::new(50, 1, 0);
        let inode = InodeItem::new(0o100755, 0, 0);
        node1.insert(BTreeItem::from_inode(1, &inode)).unwrap();

        // Create CoW duplicate at block 51, generation 2
        let mut node2 = node1.cow_duplicate(51, 2);
        assert_eq!(node2.header.block_num, 51);
        assert_eq!(node2.header.generation, 2);
        assert_eq!(node2.header.num_items, 1);
        assert!(node2.verify_checksum());

        // Modify node2: add another item
        let dir = DirEntryItem::new(99, 1, "test.txt");
        let hash = fnv1a_name_hash("test.txt");
        node2.insert(BTreeItem::from_dir_entry(1, hash, &dir)).unwrap();

        // Check node1 is untouched
        assert_eq!(node1.header.num_items, 1);
        assert_eq!(node2.header.num_items, 2);
    }
}
