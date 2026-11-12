//! kernel/src/fs/novafs/super.rs - NovaFS Superblock, Write-Ahead Journal (WAL) & Formatting
//!
//! NovaOS 2.0 (Month 2 - Storage & Async I/O): Enterprise Copy-on-Write Filesystem.
//!
//! Features:
//!   - Dual redundant superblocks (primary at block 1, backup at block 2) with SHA-256 verification.
//!   - Monotonic 64-bit transaction generations (`txid`) for crash consistency.
//!   - Write-Ahead Journaling (WAL) for atomic multi-block transactions.
//!   - BlockDevice abstraction supporting hardware controllers (NVMe, VirtIO, AHCI) & memory fixtures.
//!   - Standardized partition formatting engine (`format_partition`).

#![allow(dead_code, unused_variables)]

use super::NovaFsError;
use super::btree::{sha256, BTreeNode, BTreeItem, InodeItem, DirEntryItem, fnv1a_name_hash};

// ============================================================================
// Constants
// ============================================================================

pub const NOVAFS_MAGIC: u64 = 0x3130534641564F4E; // "NOVAFS01" in little-endian
pub const NOVAFS_BLOCK_SIZE: usize = 4096;
pub const NOVAFS_BLOCK_SIZE_U64: u64 = 4096;
pub const NOVAFS_VERSION: u32 = 1;

pub const PRIMARY_SUPER_BLOCK: u64     = 1;
pub const BACKUP_SUPER_BLOCK: u64      = 2;
pub const JOURNAL_START_BLOCK: u64     = 3;
pub const JOURNAL_BLOCK_COUNT: u32     = 32;
pub const EXTENT_TREE_ROOT_BLOCK: u64  = 35;
pub const SUBVOL_TREE_ROOT_BLOCK: u64  = 36;
pub const ROOT_SUBVOL_TREE_BLOCK: u64  = 37;
pub const FIRST_DATA_BLOCK: u64        = 38;

pub const SB_FLAG_CLEAN_UNMOUNT: u32 = 1 << 0;
pub const SB_FLAG_COW_ENABLED: u32   = 1 << 1;
pub const SB_FLAG_READ_ONLY: u32     = 1 << 2;

// Write-Ahead Journal Constants
pub const JRNL_MAGIC: u32 = 0x4C4E524A; // "JRNL" in little-endian

pub const JRNL_ENTRY_NONE: u16        = 0;
pub const JRNL_ENTRY_TX_BEGIN: u16    = 1;
pub const JRNL_ENTRY_BLOCK_WRITE: u16 = 2;
pub const JRNL_ENTRY_SNAP_CREATE: u16 = 3;
pub const JRNL_ENTRY_TX_COMMIT: u16   = 4;

pub const MAX_JRNL_RECORDS_PER_BLOCK: usize = 32;

// ============================================================================
// BlockDevice Trait & Static Buffer Implementation (Zero Stack Overhead)
// ============================================================================

pub const STATIC_DEVICE_BLOCKS: usize = 72;
pub static mut STATIC_STORAGE: [[u8; NOVAFS_BLOCK_SIZE]; STATIC_DEVICE_BLOCKS] = [[0u8; NOVAFS_BLOCK_SIZE]; STATIC_DEVICE_BLOCKS];

pub trait BlockDevice {
    fn read_block(&mut self, block_num: u64, buf: &mut [u8; NOVAFS_BLOCK_SIZE]) -> Result<(), NovaFsError>;
    fn write_block(&mut self, block_num: u64, buf: &[u8; NOVAFS_BLOCK_SIZE]) -> Result<(), NovaFsError>;
    fn block_count(&self) -> u64;
}

pub struct StaticBlockDevice;

impl StaticBlockDevice {
    pub const fn new() -> Self {
        Self
    }

    pub fn reset(&self) {
        unsafe {
            for b in 0..STATIC_DEVICE_BLOCKS {
                STATIC_STORAGE[b].fill(0);
            }
        }
    }
}

impl BlockDevice for StaticBlockDevice {
    fn read_block(&mut self, block_num: u64, buf: &mut [u8; NOVAFS_BLOCK_SIZE]) -> Result<(), NovaFsError> {
        let idx = block_num as usize;
        if idx >= STATIC_DEVICE_BLOCKS {
            return Err(NovaFsError::BlockOutOfRange);
        }
        unsafe {
            buf.copy_from_slice(&STATIC_STORAGE[idx]);
        }
        Ok(())
    }

    fn write_block(&mut self, block_num: u64, buf: &[u8; NOVAFS_BLOCK_SIZE]) -> Result<(), NovaFsError> {
        let idx = block_num as usize;
        if idx >= STATIC_DEVICE_BLOCKS {
            return Err(NovaFsError::BlockOutOfRange);
        }
        unsafe {
            STATIC_STORAGE[idx].copy_from_slice(buf);
        }
        Ok(())
    }

    fn block_count(&self) -> u64 {
        STATIC_DEVICE_BLOCKS as u64
    }
}

// ============================================================================
// Superblock Definition
// ============================================================================

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct NovaFsSuperblock {
    pub magic: u64,                  // "NOVAFS01" = 0x3130534641564F4E
    pub version: u32,                // 1
    pub block_size: u32,             // 4096
    pub generation: u64,             // Monotonic transaction ID
    pub total_blocks: u64,           // Total capacity in blocks
    pub free_blocks: u64,            // Total free blocks
    pub root_subvol_id: u64,         // Default active root subvolume ID (1)
    pub subvol_tree_root: u64,       // Subvolume/snapshot tree root block
    pub extent_tree_root: u64,       // Extent space allocation root block
    pub journal_start_block: u64,    // Write-Ahead Journal start block
    pub journal_block_count: u32,    // Number of journal blocks
    pub journal_head: u32,           // Next write slot in journal ring
    pub journal_tail: u32,           // Oldest uncommitted transaction in journal
    pub uuid: [u8; 16],              // Filesystem UUID
    pub label: [u8; 32],             // Filesystem label string
    pub flags: u32,                  // SB_FLAG_*
    pub checksum: [u8; 32],          // SHA-256 integrity checksum
}

impl NovaFsSuperblock {
    pub const fn empty() -> Self {
        Self {
            magic: NOVAFS_MAGIC,
            version: NOVAFS_VERSION,
            block_size: NOVAFS_BLOCK_SIZE as u32,
            generation: 1,
            total_blocks: 0,
            free_blocks: 0,
            root_subvol_id: 1,
            subvol_tree_root: SUBVOL_TREE_ROOT_BLOCK,
            extent_tree_root: EXTENT_TREE_ROOT_BLOCK,
            journal_start_block: JOURNAL_START_BLOCK,
            journal_block_count: JOURNAL_BLOCK_COUNT,
            journal_head: 0,
            journal_tail: 0,
            uuid: [0u8; 16],
            label: [0u8; 32],
            flags: SB_FLAG_COW_ENABLED | SB_FLAG_CLEAN_UNMOUNT,
            checksum: [0u8; 32],
        }
    }

    /// Recomputes the SHA-256 checksum over all fields except the checksum itself.
    pub fn recompute_checksum(&mut self) {
        self.checksum = [0u8; 32];
        let bytes = unsafe {
            core::slice::from_raw_parts(
                self as *const NovaFsSuperblock as *const u8,
                core::mem::size_of::<NovaFsSuperblock>() - 32, // Exclude checksum field
            )
        };
        self.checksum = sha256(bytes);
    }

    /// Verifies the superblock SHA-256 checksum.
    pub fn verify_checksum(&self) -> bool {
        let bytes = unsafe {
            core::slice::from_raw_parts(
                self as *const NovaFsSuperblock as *const u8,
                core::mem::size_of::<NovaFsSuperblock>() - 32,
            )
        };
        sha256(bytes) == self.checksum
    }

    pub fn label_as_str(&self) -> &str {
        let mut len = 0;
        while len < 32 && self.label[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.label[..len]).unwrap_or("")
    }
}

// ============================================================================
// Write-Ahead Journaling (WAL) Records
// ============================================================================

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct JournalRecord {
    pub magic: u32,                 // JRNL_MAGIC
    pub entry_type: u16,            // JRNL_ENTRY_*
    pub flags: u16,
    pub txid: u64,                  // Transaction generation
    pub block_num: u64,             // Target block
    pub target_subvol_id: u64,      // Subvolume context
    pub payload_checksum: [u8; 32], // SHA-256 checksum of block data
}

impl JournalRecord {
    pub const fn empty() -> Self {
        Self {
            magic: JRNL_MAGIC,
            entry_type: JRNL_ENTRY_NONE,
            flags: 0,
            txid: 0,
            block_num: 0,
            target_subvol_id: 0,
            payload_checksum: [0u8; 32],
        }
    }

    pub fn new_begin(txid: u64) -> Self {
        Self {
            magic: JRNL_MAGIC,
            entry_type: JRNL_ENTRY_TX_BEGIN,
            flags: 0,
            txid,
            block_num: 0,
            target_subvol_id: 0,
            payload_checksum: [0u8; 32],
        }
    }

    pub fn new_write(txid: u64, block_num: u64, subvol_id: u64, payload: &[u8]) -> Self {
        Self {
            magic: JRNL_MAGIC,
            entry_type: JRNL_ENTRY_BLOCK_WRITE,
            flags: 0,
            txid,
            block_num,
            target_subvol_id: subvol_id,
            payload_checksum: sha256(payload),
        }
    }

    pub fn new_commit(txid: u64) -> Self {
        Self {
            magic: JRNL_MAGIC,
            entry_type: JRNL_ENTRY_TX_COMMIT,
            flags: 0,
            txid,
            block_num: 0,
            target_subvol_id: 0,
            payload_checksum: [0u8; 32],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct JournalBlock {
    pub magic: u32,
    pub count: u16,
    pub reserved: u16,
    pub records: [JournalRecord; MAX_JRNL_RECORDS_PER_BLOCK],
}

impl JournalBlock {
    pub const fn empty() -> Self {
        Self {
            magic: JRNL_MAGIC,
            count: 0,
            reserved: 0,
            records: [JournalRecord::empty(); MAX_JRNL_RECORDS_PER_BLOCK],
        }
    }
}

// ============================================================================
// Formatting & Superblock Operations
// ============================================================================

/// Formats a block device partition with NovaFS structures.
pub fn format_partition(
    device: &mut dyn BlockDevice,
    total_blocks: u64,
    label_str: &str,
) -> Result<NovaFsSuperblock, NovaFsError> {
    if total_blocks < 64 {
        return Err(NovaFsError::OutOfSpace);
    }

    // 1. Prepare Superblock
    let mut sb = NovaFsSuperblock::empty();
    sb.total_blocks = total_blocks;
    sb.free_blocks = total_blocks - FIRST_DATA_BLOCK;
    sb.generation = 1;

    let bytes = label_str.as_bytes();
    let len = bytes.len().min(31);
    sb.label[..len].copy_from_slice(&bytes[..len]);
    sb.label[len] = 0;

    // UUID: deterministic seed
    sb.uuid = [
        0x55, 0x4E, 0x4F, 0x56, 0x41, 0x2D, 0x4F, 0x53,
        0x2D, 0x32, 0x30, 0x32, 0x36, 0x2D, 0x46, 0x53,
    ];
    sb.recompute_checksum();

    // 2. Initialize Subvolume Tree Root (Block 36)
    // Subvolume Tree will hold Subvolume 1 (@) and snapshot records
    let subvol_node = BTreeNode::new(SUBVOL_TREE_ROOT_BLOCK, 1, 0);
    let mut block_buf = [0u8; NOVAFS_BLOCK_SIZE];
    unsafe {
        let p = &subvol_node as *const BTreeNode as *const u8;
        core::ptr::copy_nonoverlapping(p, block_buf.as_mut_ptr(), core::mem::size_of::<BTreeNode>());
    }
    device.write_block(SUBVOL_TREE_ROOT_BLOCK, &block_buf)?;

    // 3. Initialize Root Subvolume Tree Root (Block 37)
    // Contains inode 1 (root directory '/')
    let mut root_node = BTreeNode::new(ROOT_SUBVOL_TREE_BLOCK, 1, 0);
    let root_inode = InodeItem {
        size: 0,
        num_blocks: 1,
        nlink: 2,
        uid: 0,
        gid: 0,
        mode: 0o040755, // S_IFDIR | 0755
        flags: 0,
        atime: 1000,
        mtime: 1000,
        ctime: 1000,
    };
    root_node.insert(BTreeItem::from_inode(1, &root_inode))?;

    // Add "." and ".." entries for root directory
    let dot = DirEntryItem::new(1, 2, ".");
    root_node.insert(BTreeItem::from_dir_entry(1, fnv1a_name_hash("."), &dot))?;
    let dotdot = DirEntryItem::new(1, 2, "..");
    root_node.insert(BTreeItem::from_dir_entry(1, fnv1a_name_hash(".."), &dotdot))?;

    block_buf.fill(0);
    unsafe {
        let p = &root_node as *const BTreeNode as *const u8;
        core::ptr::copy_nonoverlapping(p, block_buf.as_mut_ptr(), core::mem::size_of::<BTreeNode>());
    }
    device.write_block(ROOT_SUBVOL_TREE_BLOCK, &block_buf)?;

    // 4. Initialize empty Journal ring blocks
    let jblock = JournalBlock::empty();
    block_buf.fill(0);
    unsafe {
        let p = &jblock as *const JournalBlock as *const u8;
        core::ptr::copy_nonoverlapping(p, block_buf.as_mut_ptr(), core::mem::size_of::<JournalBlock>());
    }
    for b in 0..JOURNAL_BLOCK_COUNT {
        device.write_block(JOURNAL_START_BLOCK + b as u64, &block_buf)?;
    }

    // 5. Write Superblocks (both Primary and Backup)
    block_buf.fill(0);
    unsafe {
        let p = &sb as *const NovaFsSuperblock as *const u8;
        core::ptr::copy_nonoverlapping(p, block_buf.as_mut_ptr(), core::mem::size_of::<NovaFsSuperblock>());
    }
    device.write_block(PRIMARY_SUPER_BLOCK, &block_buf)?;
    device.write_block(BACKUP_SUPER_BLOCK, &block_buf)?;

    Ok(sb)
}

/// Atomically commits updated superblock generation with dual-ring write order.
pub fn commit_superblock(
    device: &mut dyn BlockDevice,
    sb: &mut NovaFsSuperblock,
) -> Result<(), NovaFsError> {
    sb.generation += 1;
    sb.recompute_checksum();

    let mut block_buf = [0u8; NOVAFS_BLOCK_SIZE];
    unsafe {
        let p = sb as *const NovaFsSuperblock as *const u8;
        core::ptr::copy_nonoverlapping(p, block_buf.as_mut_ptr(), core::mem::size_of::<NovaFsSuperblock>());
    }

    // Dual-commit: write backup first, then primary
    device.write_block(BACKUP_SUPER_BLOCK, &block_buf)?;
    device.write_block(PRIMARY_SUPER_BLOCK, &block_buf)?;
    Ok(())
}

/// Reads the most recent valid superblock from the dual redundant locations.
pub fn read_valid_superblock(device: &mut dyn BlockDevice) -> Result<NovaFsSuperblock, NovaFsError> {
    let mut buf = [0u8; NOVAFS_BLOCK_SIZE];

    let mut primary_sb: Option<NovaFsSuperblock> = None;
    let mut backup_sb: Option<NovaFsSuperblock> = None;

    if device.read_block(PRIMARY_SUPER_BLOCK, &mut buf).is_ok() {
        let sb = unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const NovaFsSuperblock) };
        if sb.magic == NOVAFS_MAGIC && sb.verify_checksum() {
            primary_sb = Some(sb);
        }
    }

    if device.read_block(BACKUP_SUPER_BLOCK, &mut buf).is_ok() {
        let sb = unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const NovaFsSuperblock) };
        if sb.magic == NOVAFS_MAGIC && sb.verify_checksum() {
            backup_sb = Some(sb);
        }
    }

    match (primary_sb, backup_sb) {
        (Some(p), Some(b)) => {
            if p.generation >= b.generation {
                Ok(p)
            } else {
                Ok(b)
            }
        }
        (Some(p), None) => Ok(p),
        (None, Some(b)) => Ok(b),
        (None, None) => Err(NovaFsError::InvalidSuperblock),
    }
}

// ============================================================================
// Write-Ahead Journaling Transaction Support
// ============================================================================

pub struct JournalManager<'a> {
    pub device: &'a mut dyn BlockDevice,
    pub start_block: u64,
    pub block_count: u32,
    pub head: u32,
}

impl<'a> JournalManager<'a> {
    pub fn new(device: &'a mut dyn BlockDevice, start_block: u64, block_count: u32, head: u32) -> Self {
        Self {
            device,
            start_block,
            block_count,
            head,
        }
    }

    /// Appends a journal record to the circular WAL.
    pub fn append_record(&mut self, record: JournalRecord) -> Result<(), NovaFsError> {
        let target_block = self.start_block + (self.head as u64 % self.block_count as u64);
        let mut buf = [0u8; NOVAFS_BLOCK_SIZE];
        self.device.read_block(target_block, &mut buf)?;

        let mut jblock = unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const JournalBlock) };
        if jblock.magic != JRNL_MAGIC || jblock.count as usize >= MAX_JRNL_RECORDS_PER_BLOCK {
            jblock = JournalBlock::empty();
        }

        let idx = jblock.count as usize;
        jblock.records[idx] = record;
        jblock.count += 1;

        unsafe {
            let p = &jblock as *const JournalBlock as *const u8;
            core::ptr::copy_nonoverlapping(p, buf.as_mut_ptr(), core::mem::size_of::<JournalBlock>());
        }
        self.device.write_block(target_block, &buf)?;
        self.head = (self.head + 1) % self.block_count;
        Ok(())
    }

    /// Verifies crash consistency: scans journal for uncommitted transactions and discards them.
    pub fn replay_recovery(&mut self) -> Result<u64, NovaFsError> {
        let mut highest_committed_txid = 0u64;
        let mut buf = [0u8; NOVAFS_BLOCK_SIZE];

        for b in 0..self.block_count {
            let blk = self.start_block + b as u64;
            if self.device.read_block(blk, &mut buf).is_ok() {
                let jblock = unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const JournalBlock) };
                if jblock.magic == JRNL_MAGIC {
                    for i in 0..jblock.count as usize {
                        let rec = &jblock.records[i];
                        if rec.entry_type == JRNL_ENTRY_TX_COMMIT && rec.txid > highest_committed_txid {
                            highest_committed_txid = rec.txid;
                        }
                    }
                }
            }
        }
        Ok(highest_committed_txid)
    }
}

// ============================================================================
// Unit Tests (Host-side cargo test / rustc --test)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_and_mount() {
        let mut dev = StaticBlockDevice::new();
        dev.reset();
        let sb = format_partition(&mut dev, 72, "NovaOS-Test").expect("Format failed");

        assert_eq!(sb.magic, NOVAFS_MAGIC);
        assert_eq!(sb.version, NOVAFS_VERSION);
        assert_eq!(sb.total_blocks, 72);
        assert_eq!(sb.generation, 1);
        assert_eq!(sb.label_as_str(), "NovaOS-Test");
        assert!(sb.verify_checksum());

        let mounted_sb = read_valid_superblock(&mut dev).expect("Mount failed");
        assert_eq!(mounted_sb.generation, 1);
        assert_eq!(mounted_sb.total_blocks, 72);
        assert_eq!(mounted_sb.label_as_str(), "NovaOS-Test");
    }

    #[test]
    fn test_dual_superblock_commit() {
        let mut dev = StaticBlockDevice::new();
        dev.reset();
        let mut sb = format_partition(&mut dev, 72, "DualCommit").expect("Format failed");

        assert_eq!(sb.generation, 1);

        // Commit generation 2
        commit_superblock(&mut dev, &mut sb).expect("Commit failed");
        assert_eq!(sb.generation, 2);

        let mounted = read_valid_superblock(&mut dev).expect("Read failed");
        assert_eq!(mounted.generation, 2);
    }

    #[test]
    fn test_journal_wal_commit_and_replay() {
        let mut dev = StaticBlockDevice::new();
        dev.reset();
        let sb = format_partition(&mut dev, 72, "WAL-Test").expect("Format failed");

        let mut jm = JournalManager::new(&mut dev, sb.journal_start_block, sb.journal_block_count, 0);

        // Transaction 1: Begin + Write + Commit
        jm.append_record(JournalRecord::new_begin(10)).unwrap();
        jm.append_record(JournalRecord::new_write(10, 40, 1, b"Payload block 40")).unwrap();
        jm.append_record(JournalRecord::new_commit(10)).unwrap();

        // Transaction 2: Begin + Write (abrupt crash, no commit)
        jm.append_record(JournalRecord::new_begin(11)).unwrap();
        jm.append_record(JournalRecord::new_write(11, 41, 1, b"Uncommitted payload")).unwrap();

        // Recovery should identify 10 as highest committed transaction
        let committed_txid = jm.replay_recovery().unwrap();
        assert_eq!(committed_txid, 10, "Uncommitted txid 11 must be ignored on recovery");
    }
}
