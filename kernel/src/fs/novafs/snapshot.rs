//! kernel/src/fs/novafs/snapshot.rs - NovaFS Subvolumes, Atomic Snapshots & Rollback Engine
//!
//! NovaOS 2.0 (Month 2 - Storage & Async I/O): Enterprise Copy-on-Write Filesystem.
//!
//! Features:
//!   - Subvolume management for container root filesystems and multi-tenant OS trees.
//!   - Instantaneous O(1) zero-cost atomic snapshots (sub-second creation).
//!   - Copy-on-Write (CoW) divergence: historical snapshots remain immutable when parent diverges.
//!   - Writable subvolume branching for safe OS update validation.
//!   - Instantaneous snapshot rollback restoring subvolume root pointer to historical generation.

#![allow(dead_code, unused_variables)]

use super::NovaFsError;

// ============================================================================
// Constants
// ============================================================================

pub const SUBVOL_FLAG_READONLY: u32 = 1 << 0;
pub const SUBVOL_FLAG_SNAPSHOT: u32 = 1 << 1;
pub const SUBVOL_FLAG_ACTIVE: u32   = 1 << 2;
pub const SUBVOL_FLAG_DELETED: u32  = 1 << 3;

pub const MAX_SUBVOLUMES: usize = 32;

// ============================================================================
// Subvolume Record
// ============================================================================

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SubvolumeRecord {
    pub id: u64,             // Subvolume unique ID (1 = root '@')
    pub parent_id: u64,      // Parent ID (0 for root, or origin subvol for snapshots)
    pub root_block: u64,     // Physical disk block of the root B-Tree node
    pub generation: u64,     // Monotonic generation at snapshot creation
    pub ctime: u64,          // Creation timestamp
    pub flags: u32,          // SUBVOL_FLAG_*
    pub name_len: u8,        // Subvolume name length
    pub name: [u8; 63],      // Subvolume name (e.g. "@", "@home", "snap-20261112")
}

impl SubvolumeRecord {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            parent_id: 0,
            root_block: 0,
            generation: 0,
            ctime: 0,
            flags: 0,
            name_len: 0,
            name: [0u8; 63],
        }
    }

    pub fn new(id: u64, parent_id: u64, root_block: u64, generation: u64, flags: u32, name_str: &str) -> Self {
        let bytes = name_str.as_bytes();
        let len = bytes.len().min(63);
        let mut name = [0u8; 63];
        let mut i = 0;
        while i < len {
            name[i] = bytes[i];
            i += 1;
        }
        Self {
            id,
            parent_id,
            root_block,
            generation,
            ctime: 1000,
            flags: flags | SUBVOL_FLAG_ACTIVE,
            name_len: len as u8,
            name,
        }
    }

    pub fn name_as_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("")
    }

    pub fn is_readonly(&self) -> bool {
        (self.flags & SUBVOL_FLAG_READONLY) != 0
    }

    pub fn is_snapshot(&self) -> bool {
        (self.flags & SUBVOL_FLAG_SNAPSHOT) != 0
    }

    pub fn is_active(&self) -> bool {
        (self.flags & SUBVOL_FLAG_ACTIVE) != 0 && (self.flags & SUBVOL_FLAG_DELETED) == 0
    }
}

// ============================================================================
// Subvolume Registry
// ============================================================================

pub struct SubvolumeRegistry {
    pub subvolumes: [SubvolumeRecord; MAX_SUBVOLUMES],
    pub count: usize,
}

impl SubvolumeRegistry {
    pub const fn new() -> Self {
        Self {
            subvolumes: [SubvolumeRecord::empty(); MAX_SUBVOLUMES],
            count: 0,
        }
    }

    /// Registers the default root subvolume ('@' with ID 1).
    pub fn init_root(&mut self, root_block: u64, generation: u64) {
        self.subvolumes[0] = SubvolumeRecord::new(
            1,
            0,
            root_block,
            generation,
            SUBVOL_FLAG_ACTIVE,
            "@",
        );
        self.count = 1;
    }

    /// Registers a new or updated subvolume record.
    pub fn register(&mut self, record: SubvolumeRecord) -> Result<(), NovaFsError> {
        // If ID exists, update it
        for i in 0..self.count {
            if self.subvolumes[i].id == record.id {
                self.subvolumes[i] = record;
                return Ok(());
            }
        }
        if self.count >= MAX_SUBVOLUMES {
            return Err(NovaFsError::OutOfSpace);
        }
        self.subvolumes[self.count] = record;
        self.count += 1;
        Ok(())
    }

    /// Looks up a subvolume record by unique ID.
    pub fn get_by_id(&self, id: u64) -> Option<&SubvolumeRecord> {
        for i in 0..self.count {
            if self.subvolumes[i].id == id && self.subvolumes[i].is_active() {
                return Some(&self.subvolumes[i]);
            }
        }
        None
    }

    /// Looks up a mutable subvolume record by unique ID.
    pub fn get_by_id_mut(&mut self, id: u64) -> Option<&mut SubvolumeRecord> {
        for i in 0..self.count {
            if self.subvolumes[i].id == id && self.subvolumes[i].is_active() {
                return Some(&mut self.subvolumes[i]);
            }
        }
        None
    }

    /// Looks up a subvolume record by name string.
    pub fn get_by_name(&self, name: &str) -> Option<&SubvolumeRecord> {
        for i in 0..self.count {
            if self.subvolumes[i].is_active() && self.subvolumes[i].name_as_str() == name {
                return Some(&self.subvolumes[i]);
            }
        }
        None
    }

    /// Instantaneous O(1) Atomic Snapshot Creation.
    ///
    /// Copies zero data blocks: the snapshot subvolume header directly points
    /// to the current root B-Tree node of the source subvolume!
    pub fn create_snapshot(
        &mut self,
        src_subvol_id: u64,
        snap_id: u64,
        snap_name: &str,
        generation: u64,
        read_only: bool,
    ) -> Result<SubvolumeRecord, NovaFsError> {
        let src_root = match self.get_by_id(src_subvol_id) {
            Some(s) => s.root_block,
            None => return Err(NovaFsError::SubvolumeNotFound),
        };

        let mut flags = SUBVOL_FLAG_SNAPSHOT;
        if read_only {
            flags |= SUBVOL_FLAG_READONLY;
        }

        let snap_rec = SubvolumeRecord::new(
            snap_id,
            src_subvol_id,
            src_root, // Points to exact same B-tree root!
            generation,
            flags,
            snap_name,
        );

        self.register(snap_rec)?;
        Ok(snap_rec)
    }

    /// Creates a new writable subvolume branch (for containers, /home, etc.).
    pub fn create_subvolume(
        &mut self,
        new_id: u64,
        root_block: u64,
        generation: u64,
        name: &str,
    ) -> Result<SubvolumeRecord, NovaFsError> {
        let rec = SubvolumeRecord::new(
            new_id,
            0,
            root_block,
            generation,
            SUBVOL_FLAG_ACTIVE,
            name,
        );
        self.register(rec)?;
        Ok(rec)
    }

    /// Instantaneous Subvolume Rollback.
    ///
    /// Atomically points the target subvolume root block back to the snapshot's
    /// historical B-Tree root block.
    pub fn rollback_to_snapshot(
        &mut self,
        target_subvol_id: u64,
        snap_id: u64,
    ) -> Result<(), NovaFsError> {
        let snap_root = match self.get_by_id(snap_id) {
            Some(snap) => {
                if !snap.is_snapshot() {
                    return Err(NovaFsError::InvalidPath);
                }
                snap.root_block
            }
            None => return Err(NovaFsError::SubvolumeNotFound),
        };

        match self.get_by_id_mut(target_subvol_id) {
            Some(target) => {
                if target.is_readonly() {
                    return Err(NovaFsError::SubvolumeReadOnly);
                }
                target.root_block = snap_root;
                Ok(())
            }
            None => Err(NovaFsError::SubvolumeNotFound),
        }
    }

    /// Deletes a subvolume or snapshot by marking it deleted.
    pub fn delete_subvolume(&mut self, subvol_id: u64) -> Result<(), NovaFsError> {
        if subvol_id == 1 {
            // Cannot delete root subvolume
            return Err(NovaFsError::SubvolumeReadOnly);
        }
        match self.get_by_id_mut(subvol_id) {
            Some(s) => {
                s.flags |= SUBVOL_FLAG_DELETED;
                s.flags &= !SUBVOL_FLAG_ACTIVE;
                Ok(())
            }
            None => Err(NovaFsError::SubvolumeNotFound),
        }
    }
}

// ============================================================================
// Unit Tests (Host-side cargo test / rustc --test)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subvolume_creation_and_lookup() {
        let mut reg = SubvolumeRegistry::new();
        reg.init_root(37, 1);

        let root = reg.get_by_id(1).expect("Root subvolume not found");
        assert_eq!(root.name_as_str(), "@");
        assert_eq!(root.root_block, 37);
        assert!(!root.is_readonly());
        assert!(!root.is_snapshot());

        let looked_up = reg.get_by_name("@").expect("Lookup by name failed");
        assert_eq!(looked_up.id, 1);
    }

    #[test]
    fn test_instant_snapshot_creation() {
        let mut reg = SubvolumeRegistry::new();
        reg.init_root(37, 1);

        // Create atomic read-only snapshot
        let snap = reg.create_snapshot(1, 100, "@snap1", 1, true).expect("Snapshot failed");
        assert_eq!(snap.id, 100);
        assert_eq!(snap.parent_id, 1);
        assert_eq!(snap.root_block, 37, "Snapshot must share the exact same root block in O(1)");
        assert!(snap.is_readonly());
        assert!(snap.is_snapshot());

        let retrieved = reg.get_by_name("@snap1").expect("Snapshot not found");
        assert_eq!(retrieved.id, 100);
    }

    #[test]
    fn test_cow_divergence_between_subvol_and_snapshot() {
        let mut reg = SubvolumeRegistry::new();
        reg.init_root(37, 1);

        // Take snapshot at block 37
        reg.create_snapshot(1, 100, "@snap_init", 1, true).unwrap();

        // Mutate root subvolume: CoW allocates a new root block 45
        {
            let root = reg.get_by_id_mut(1).unwrap();
            root.root_block = 45;
        }

        // Snapshot MUST remain pointing to the original block 37
        let snap = reg.get_by_id(100).unwrap();
        assert_eq!(snap.root_block, 37, "Snapshot root block must never change when parent diverges");

        // Root subvolume points to the new block 45
        let root = reg.get_by_id(1).unwrap();
        assert_eq!(root.root_block, 45);
    }

    #[test]
    fn test_snapshot_rollback() {
        let mut reg = SubvolumeRegistry::new();
        reg.init_root(37, 1);

        // Take snapshot at block 37
        reg.create_snapshot(1, 100, "@snap_clean", 1, true).unwrap();

        // Root modified to block 50
        reg.get_by_id_mut(1).unwrap().root_block = 50;
        assert_eq!(reg.get_by_id(1).unwrap().root_block, 50);

        // Roll back to snapshot 100
        reg.rollback_to_snapshot(1, 100).expect("Rollback failed");
        assert_eq!(reg.get_by_id(1).unwrap().root_block, 37, "Rollback must restore root block to snapshot");
    }

    #[test]
    fn test_writable_subvolume_branch() {
        let mut reg = SubvolumeRegistry::new();
        reg.init_root(37, 1);

        // Branch a writable container root subvolume
        let sub = reg.create_snapshot(1, 200, "@container_root", 1, false).expect("Branch failed");
        assert!(!sub.is_readonly(), "Container branch should be writable");
        assert_eq!(sub.root_block, 37);

        // Modifying container branch to block 60 does not affect root (block 37)
        reg.get_by_id_mut(200).unwrap().root_block = 60;
        assert_eq!(reg.get_by_id(1).unwrap().root_block, 37);
        assert_eq!(reg.get_by_id(200).unwrap().root_block, 60);
    }
}
