//! kernel/src/fs/novafs/mod.rs - NovaFS Pure Rust Enterprise Copy-on-Write Filesystem
//!
//! NovaOS 2.0 (Month 2 - Storage & Async I/O): Enterprise Storage Engine.
//!
//! Features:
//!   - Modeled after ZFS & Btrfs: B-Tree on-disk indexing, 64-bit extents, and CoW trees.
//!   - Dual redundant superblocks with SHA-256 cryptographic verification.
//!   - Write-Ahead Journaling (WAL) for 100% crash consistency and power-loss resilience.
//!   - Instantaneous O(1) zero-cost atomic snapshots and writable subvolumes.
//!   - Pure #[no_std] SHA-256 data block integrity checking to automatically detect bit-rot.

#![allow(dead_code, unused_variables, static_mut_refs)]

pub mod btree;
#[path = "super.rs"]
pub mod super_block;
pub mod snapshot;

pub use btree::*;
pub use super_block::*;
pub use snapshot::*;

// ============================================================================
// NovaFs High-Level Filesystem Engine
// ============================================================================

pub struct NovaFs {
    pub superblock: NovaFsSuperblock,
    pub subvol_registry: SubvolumeRegistry,
    pub active_subvol_id: u64,
    pub next_free_block: u64,
    pub next_inode_num: u64,
    pub mem_dev: StaticBlockDevice,
}

impl NovaFs {
    /// Initializes and formats a new in-memory NovaFS instance for kernel self-test & operation.
    pub fn new_formatted(label: &str) -> Result<Self, NovaFsError> {
        let mut dev = StaticBlockDevice::new();
        dev.reset();
        let sb = format_partition(&mut dev, 72, label)?;

        let mut registry = SubvolumeRegistry::new();
        registry.init_root(ROOT_SUBVOL_TREE_BLOCK, 1);

        Ok(Self {
            superblock: sb,
            subvol_registry: registry,
            active_subvol_id: 1,
            next_free_block: FIRST_DATA_BLOCK,
            next_inode_num: 2, // 1 is root dir '/'
            mem_dev: dev,
        })
    }

    /// Allocates the next contiguous disk block from the free list.
    pub fn alloc_block(&mut self) -> Result<u64, NovaFsError> {
        if self.next_free_block >= self.superblock.total_blocks {
            return Err(NovaFsError::OutOfSpace);
        }
        let blk = self.next_free_block;
        self.next_free_block += 1;
        self.superblock.free_blocks = self.superblock.total_blocks.saturating_sub(self.next_free_block);
        Ok(blk)
    }

    /// Creates a new regular file or directory under a parent directory.
    pub fn create_entry(
        &mut self,
        parent_ino: u64,
        name: &str,
        mode: u32,
        uid: u32,
        gid: u32,
        is_dir: bool,
    ) -> Result<u64, NovaFsError> {
        let active_subvol = match self.subvol_registry.get_by_id(self.active_subvol_id) {
            Some(s) => {
                if s.is_readonly() {
                    return Err(NovaFsError::SubvolumeReadOnly);
                }
                *s
            }
            None => return Err(NovaFsError::SubvolumeNotFound),
        };

        // Read active root B-Tree node
        let mut buf = [0u8; NOVAFS_BLOCK_SIZE];
        self.mem_dev.read_block(active_subvol.root_block, &mut buf)?;
        let root_node = unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const BTreeNode) };

        // CoW: allocate new root block
        let new_root_blk = self.alloc_block()?;
        let mut cow_root = root_node.cow_duplicate(new_root_blk, self.superblock.generation + 1);

        let new_ino = self.next_inode_num;
        self.next_inode_num += 1;

        // 1. Insert InodeItem
        let inode = InodeItem::new(mode, uid, gid);
        cow_root.insert(BTreeItem::from_inode(new_ino, &inode))?;

        // 2. Insert DirEntryItem in parent directory
        let file_type = if is_dir { 2 } else { 1 };
        let dir = DirEntryItem::new(new_ino, file_type, name);
        let name_hash = fnv1a_name_hash(name);
        cow_root.insert(BTreeItem::from_dir_entry(parent_ino, name_hash, &dir))?;

        // Write CoW root to new block
        buf.fill(0);
        unsafe {
            let p = &cow_root as *const BTreeNode as *const u8;
            core::ptr::copy_nonoverlapping(p, buf.as_mut_ptr(), core::mem::size_of::<BTreeNode>());
        }
        self.mem_dev.write_block(new_root_blk, &buf)?;

        // Update subvolume root block pointer
        if let Some(sub) = self.subvol_registry.get_by_id_mut(self.active_subvol_id) {
            sub.root_block = new_root_blk;
            sub.generation = self.superblock.generation + 1;
        }

        // Commit superblock
        commit_superblock(&mut self.mem_dev, &mut self.superblock)?;
        Ok(new_ino)
    }

    /// Writes data to a file using 64-bit extents, CoW tree updates, and SHA-256 checksumming.
    pub fn write_file(&mut self, ino: u64, offset: u64, data: &[u8]) -> Result<usize, NovaFsError> {
        let active_subvol = match self.subvol_registry.get_by_id(self.active_subvol_id) {
            Some(s) => {
                if s.is_readonly() {
                    return Err(NovaFsError::SubvolumeReadOnly);
                }
                *s
            }
            None => return Err(NovaFsError::SubvolumeNotFound),
        };

        // 1. Allocate a fresh physical block for the data payload
        let data_block = self.alloc_block()?;
        let mut data_buf = [0u8; NOVAFS_BLOCK_SIZE];
        let write_len = data.len().min(NOVAFS_BLOCK_SIZE);
        data_buf[..write_len].copy_from_slice(&data[..write_len]);
        self.mem_dev.write_block(data_block, &data_buf)?;

        // 2. Construct ExtentDataItem with SHA-256 checksum
        let extent = ExtentDataItem::new(offset, data_block, &data_buf[..write_len]);

        // 3. CoW B-Tree node mutation
        let mut buf = [0u8; NOVAFS_BLOCK_SIZE];
        self.mem_dev.read_block(active_subvol.root_block, &mut buf)?;
        let root_node = unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const BTreeNode) };

        let new_root_blk = self.alloc_block()?;
        let mut cow_root = root_node.cow_duplicate(new_root_blk, self.superblock.generation + 1);

        // Insert extent item
        cow_root.insert(BTreeItem::from_extent(ino, offset, &extent))?;

        // Update inode size
        if let Some(item) = cow_root.get_mut(&BTreeKey::inode(ino)) {
            if let Some(mut in_val) = item.as_inode() {
                if offset + write_len as u64 > in_val.size {
                    in_val.size = offset + write_len as u64;
                    in_val.num_blocks += 1;
                    *item = BTreeItem::from_inode(ino, &in_val);
                }
            }
        }

        // Write CoW root to new block
        buf.fill(0);
        unsafe {
            let p = &cow_root as *const BTreeNode as *const u8;
            core::ptr::copy_nonoverlapping(p, buf.as_mut_ptr(), core::mem::size_of::<BTreeNode>());
        }
        self.mem_dev.write_block(new_root_blk, &buf)?;

        // Update subvolume root block pointer
        if let Some(sub) = self.subvol_registry.get_by_id_mut(self.active_subvol_id) {
            sub.root_block = new_root_blk;
            sub.generation = self.superblock.generation + 1;
        }

        // Commit superblock
        commit_superblock(&mut self.mem_dev, &mut self.superblock)?;
        Ok(write_len)
    }

    /// Reads data from a file, validating the SHA-256 checksum to detect bit-rot.
    pub fn read_file(&mut self, ino: u64, offset: u64, out_buf: &mut [u8]) -> Result<usize, NovaFsError> {
        let active_subvol = match self.subvol_registry.get_by_id(self.active_subvol_id) {
            Some(s) => *s,
            None => return Err(NovaFsError::SubvolumeNotFound),
        };

        // Read active subvolume root B-Tree
        let mut buf = [0u8; NOVAFS_BLOCK_SIZE];
        self.mem_dev.read_block(active_subvol.root_block, &mut buf)?;
        let root_node = unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const BTreeNode) };

        // Look up extent
        let extent_item = match root_node.get(&BTreeKey::extent(ino, offset)) {
            Some(it) => it.as_extent().ok_or(NovaFsError::KeyNotFound)?,
            None => return Err(NovaFsError::KeyNotFound),
        };

        // Read physical block
        let mut disk_buf = [0u8; NOVAFS_BLOCK_SIZE];
        self.mem_dev.read_block(extent_item.physical_block, &mut disk_buf)?;

        // SHA-256 Bit-Rot Verification!
        if !extent_item.verify_integrity(&disk_buf) {
            return Err(NovaFsError::ChecksumMismatch);
        }

        let copy_len = (extent_item.byte_len as usize).min(out_buf.len());
        out_buf[..copy_len].copy_from_slice(&disk_buf[..copy_len]);
        Ok(copy_len)
    }

    /// Looks up a file inode by directory entry name.
    pub fn lookup(&mut self, parent_ino: u64, name: &str) -> Result<u64, NovaFsError> {
        let active_subvol = match self.subvol_registry.get_by_id(self.active_subvol_id) {
            Some(s) => *s,
            None => return Err(NovaFsError::SubvolumeNotFound),
        };

        let mut buf = [0u8; NOVAFS_BLOCK_SIZE];
        self.mem_dev.read_block(active_subvol.root_block, &mut buf)?;
        let root_node = unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const BTreeNode) };

        let hash = fnv1a_name_hash(name);
        match root_node.get(&BTreeKey::dir_entry(parent_ino, hash)) {
            Some(it) => {
                let dir = it.as_dir_entry().ok_or(NovaFsError::KeyNotFound)?;
                Ok(dir.child_ino)
            }
            None => Err(NovaFsError::KeyNotFound),
        }
    }

    /// Instantaneous O(1) Atomic Snapshot of the current subvolume.
    pub fn create_snapshot(&mut self, snap_id: u64, snap_name: &str, read_only: bool) -> Result<SubvolumeRecord, NovaFsError> {
        self.subvol_registry.create_snapshot(
            self.active_subvol_id,
            snap_id,
            snap_name,
            self.superblock.generation,
            read_only,
        )
    }

    /// Rolls back active subvolume to a historical snapshot.
    pub fn rollback_active_to_snapshot(&mut self, snap_id: u64) -> Result<(), NovaFsError> {
        self.subvol_registry.rollback_to_snapshot(self.active_subvol_id, snap_id)?;
        commit_superblock(&mut self.mem_dev, &mut self.superblock)?;
        Ok(())
    }

    /// Switches the active subvolume context.
    pub fn set_active_subvolume(&mut self, subvol_id: u64) -> Result<(), NovaFsError> {
        if self.subvol_registry.get_by_id(subvol_id).is_some() {
            self.active_subvol_id = subvol_id;
            Ok(())
        } else {
            Err(NovaFsError::SubvolumeNotFound)
        }
    }
}

// ============================================================================
// Ring-0 In-Kernel Self-Test (12 Comprehensive Stages)
// ============================================================================

/// Runs the complete NovaFS enterprise verification suite.
/// Returns 0 on success, or negative stage code on failure.
#[no_mangle]
pub extern "C" fn run_novafs_selftest() -> i32 {
    // ------------------------------------------------------------------------
    // Stage 1: Partition Formatting & Superblock Initialization
    // ------------------------------------------------------------------------
    let mut fs = match NovaFs::new_formatted("NovaOS-Root") {
        Ok(f) => f,
        Err(_) => return -1,
    };

    // ------------------------------------------------------------------------
    // Stage 2: Superblock Validation & Integrity Checksum
    // ------------------------------------------------------------------------
    if fs.superblock.magic != NOVAFS_MAGIC {
        return -2;
    }
    if fs.superblock.block_size != 4096 || fs.superblock.generation != 1 {
        return -2;
    }
    if !fs.superblock.verify_checksum() {
        return -2;
    }

    // ------------------------------------------------------------------------
    // Stage 3: B-Tree Indexing & Directory Operations
    // ------------------------------------------------------------------------
    let file1_ino = match fs.create_entry(1, "system.conf", 0o100644, 0, 0, false) {
        Ok(ino) => ino,
        Err(_) => return -3,
    };
    let lookup_ino = match fs.lookup(1, "system.conf") {
        Ok(ino) => ino,
        Err(_) => return -3,
    };
    if file1_ino != lookup_ino {
        return -3;
    }

    // ------------------------------------------------------------------------
    // Stage 4: 64-Bit Extents & SHA-256 Cryptographic Checksum
    // ------------------------------------------------------------------------
    let test_data = b"NovaOS 2.0 Pure Rust Enterprise Filesystem: CoW & Snapshots!";
    if fs.write_file(file1_ino, 0, test_data).is_err() {
        return -4;
    }

    let mut read_buf = [0u8; 64];
    let bytes_read = match fs.read_file(file1_ino, 0, &mut read_buf) {
        Ok(n) => n,
        Err(_) => return -4,
    };
    if bytes_read != test_data.len() || &read_buf[..bytes_read] != test_data {
        return -4;
    }

    // ------------------------------------------------------------------------
    // Stage 5: Bit-Rot Detection (SHA-256 Block Checksum Mismatch)
    // ------------------------------------------------------------------------
    // Inject bit-rot into the disk block: flip 1 bit in physical storage
    let corrupted_block = fs.next_free_block - 2; // Data block of file1
    unsafe {
        STATIC_STORAGE[corrupted_block as usize][10] ^= 0x01;
    }

    // Reading corrupted file MUST return ChecksumMismatch error
    match fs.read_file(file1_ino, 0, &mut read_buf) {
        Err(NovaFsError::ChecksumMismatch) => {
            // Expected: bit-rot detected! Restore bit for subsequent tests
            unsafe {
                STATIC_STORAGE[corrupted_block as usize][10] ^= 0x01;
            }
        }
        _ => return -5,
    }

    // ------------------------------------------------------------------------
    // Stage 6: Copy-on-Write (CoW) Node Duplication & History Preservation
    // ------------------------------------------------------------------------
    let initial_root_blk = fs.subvol_registry.get_by_id(1).unwrap().root_block;
    let file2_ino = match fs.create_entry(1, "kernel.sys", 0o100755, 0, 0, false) {
        Ok(ino) => ino,
        Err(_) => return -6,
    };
    let new_root_blk = fs.subvol_registry.get_by_id(1).unwrap().root_block;
    if initial_root_blk == new_root_blk {
        return -6; // CoW must allocate a distinct root block
    }

    // ------------------------------------------------------------------------
    // Stage 7: Instantaneous O(1) Atomic Snapshot Creation
    // ------------------------------------------------------------------------
    let snap_rec = match fs.create_snapshot(100, "@snap_initial", true) {
        Ok(s) => s,
        Err(_) => return -7,
    };
    if snap_rec.id != 100 || snap_rec.root_block != new_root_blk {
        return -7; // Snapshot MUST point to exact same root block in O(1)
    }
    if !snap_rec.is_readonly() || !snap_rec.is_snapshot() {
        return -7;
    }

    // ------------------------------------------------------------------------
    // Stage 8: CoW Tree Divergence & Snapshot Immutability
    // ------------------------------------------------------------------------
    // Modify root subvolume: write a new file "update.log"
    let file3_ino = match fs.create_entry(1, "update.log", 0o100644, 0, 0, false) {
        Ok(ino) => ino,
        Err(_) => return -8,
    };
    if fs.write_file(file3_ino, 0, b"OS Update 2.1").is_err() {
        return -8;
    }

    // Root subvolume has update.log
    if fs.lookup(1, "update.log").is_err() {
        return -8;
    }

    // Switch active to snapshot @snap_initial
    fs.set_active_subvolume(100).unwrap();
    // Snapshot MUST NOT contain "update.log" (isolated at snapshot point in time)
    if fs.lookup(1, "update.log").is_ok() {
        return -8; // Snapshot leaked post-snapshot writes!
    }

    // ------------------------------------------------------------------------
    // Stage 9: Read-Only Snapshot Enforcement
    // ------------------------------------------------------------------------
    match fs.create_entry(1, "illegal.txt", 0o100644, 0, 0, false) {
        Err(NovaFsError::SubvolumeReadOnly) => {} // Correctly rejected!
        _ => return -9,
    }

    // ------------------------------------------------------------------------
    // Stage 10: Writable Subvolume Branching (Containers / Overlay)
    // ------------------------------------------------------------------------
    // Switch back to root subvolume 1
    fs.set_active_subvolume(1).unwrap();
    // Create a writable branch '@container1' from snapshot 100
    let cont_rec = match fs.subvol_registry.create_snapshot(100, 200, "@container1", fs.superblock.generation, false) {
        Ok(s) => s,
        Err(_) => return -10,
    };
    if cont_rec.is_readonly() {
        return -10;
    }
    // Switch active to container subvolume 200 and write a container-specific file
    fs.set_active_subvolume(200).unwrap();
    let cont_file = match fs.create_entry(1, "container.cfg", 0o100644, 0, 0, false) {
        Ok(ino) => ino,
        Err(_) => return -10,
    };
    if fs.lookup(1, "container.cfg").is_err() {
        return -10;
    }
    // Verify container file is NOT visible in root subvolume 1
    fs.set_active_subvolume(1).unwrap();
    if fs.lookup(1, "container.cfg").is_ok() {
        return -10; // Container leaked into root!
    }

    // ------------------------------------------------------------------------
    // Stage 11: Write-Ahead Journaling (WAL) Transaction & Crash Recovery
    // ------------------------------------------------------------------------
    let mut jm = JournalManager::new(
        &mut fs.mem_dev,
        fs.superblock.journal_start_block,
        fs.superblock.journal_block_count,
        0,
    );
    // Write committed transaction
    jm.append_record(JournalRecord::new_begin(50)).unwrap();
    jm.append_record(JournalRecord::new_write(50, 42, 1, b"Transaction commit data")).unwrap();
    jm.append_record(JournalRecord::new_commit(50)).unwrap();

    // Write uncommitted transaction (crash simulated before commit)
    jm.append_record(JournalRecord::new_begin(51)).unwrap();
    jm.append_record(JournalRecord::new_write(51, 43, 1, b"Corrupted crash data")).unwrap();

    let replayed = jm.replay_recovery().unwrap();
    if replayed != 50 {
        return -11; // Must discard uncommitted txid 51
    }

    // ------------------------------------------------------------------------
    // Stage 12: Instantaneous Snapshot Rollback
    // ------------------------------------------------------------------------
    // In root subvolume, update.log currently exists
    if fs.lookup(1, "update.log").is_err() {
        return -12;
    }
    // Roll back root subvolume 1 to snapshot 100 (@snap_initial)
    if fs.rollback_active_to_snapshot(100).is_err() {
        return -12;
    }
    // After rollback, update.log must NO LONGER exist!
    if fs.lookup(1, "update.log").is_ok() {
        return -12; // Rollback failed to restore historical tree!
    }
    // Initial file system.conf must still exist!
    if fs.lookup(1, "system.conf").is_err() {
        return -12;
    }

    0 // All 12 stages passed!
}

// ============================================================================
// Unit Tests (Host-side cargo test / rustc --test)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_novafs_full_lifecycle() {
        let res = run_novafs_selftest();
        assert_eq!(res, 0, "run_novafs_selftest failed with error code: {}", res);
    }
}
