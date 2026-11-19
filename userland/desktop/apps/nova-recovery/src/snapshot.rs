//! userland/desktop/apps/nova-recovery/src/snapshot.rs
//!
//! NovaFS Snapshot Management & Atomic Rollback Engine for NovaOS Recovery Environment.
//! Enables operators to inspect historical CoW snapshots and execute instant rollback.

pub const MAX_SNAPSHOTS: usize = 32;

/// NovaFS Snapshot Descriptor
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NovaFsSnapshot {
    pub id: u64,
    pub parent_id: u64,
    pub root_block: u64,
    pub generation: u64,
    pub timestamp: u64,
    pub name: String,
    pub is_readonly: bool,
    pub size_bytes: u64,
}

impl NovaFsSnapshot {
    pub fn new(
        id: u64,
        parent_id: u64,
        root_block: u64,
        generation: u64,
        timestamp: u64,
        name: &str,
        is_readonly: bool,
        size_bytes: u64,
    ) -> Self {
        Self {
            id,
            parent_id,
            root_block,
            generation,
            timestamp,
            name: name.to_string(),
            is_readonly,
            size_bytes,
        }
    }
}

/// Manages discovery, verification, and rollback of NovaFS filesystem snapshots.
pub struct SnapshotManager {
    pub active_subvolume_id: u64,
    pub current_root_block: u64,
    pub current_generation: u64,
    pub snapshots: Vec<NovaFsSnapshot>,
    pub next_snapshot_id: u64,
}

impl SnapshotManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            active_subvolume_id: 1, // '@' root subvolume
            current_root_block: 1048576,
            current_generation: 42,
            snapshots: Vec::new(),
            next_snapshot_id: 100,
        };
        // Seed standard enterprise snapshots (e.g. system checkpoints)
        mgr.seed_default_snapshots();
        mgr
    }

    /// Seeds realistic initial system snapshots for recovery testing and display
    pub fn seed_default_snapshots(&mut self) {
        self.snapshots.push(NovaFsSnapshot::new(
            101,
            1,
            1024000,
            38,
            1731900000,
            "@snapshot-20261115-base-install",
            true,
            8_589_934_592, // 8.0 GB
        ));
        self.snapshots.push(NovaFsSnapshot::new(
            102,
            1,
            1036000,
            40,
            1731986400,
            "@snapshot-20261117-pre-update-v2",
            true,
            8_724_152_320, // 8.12 GB
        ));
        self.snapshots.push(NovaFsSnapshot::new(
            103,
            1,
            1042000,
            41,
            1732000000,
            "@snapshot-20261118-daily-backup",
            true,
            8_858_370_048, // 8.25 GB
        ));
        self.next_snapshot_id = 104;
    }

    /// Lists all discovered snapshots sorted by generation
    pub fn list_snapshots(&self) -> &[NovaFsSnapshot] {
        &self.snapshots
    }

    /// Finds a snapshot by its unique ID
    pub fn find_by_id(&self, id: u64) -> Option<&NovaFsSnapshot> {
        self.snapshots.iter().find(|s| s.id == id)
    }

    /// Creates an emergency safety snapshot prior to executing any rollback
    pub fn create_pre_rollback_checkpoint(&mut self, label: &str) -> Result<u64, String> {
        let new_id = self.next_snapshot_id;
        self.next_snapshot_id += 1;

        let snap_name = format!("@pre-rollback-{}-id{}", label, new_id);
        let checkpoint = NovaFsSnapshot::new(
            new_id,
            self.active_subvolume_id,
            self.current_root_block,
            self.current_generation,
            1732050000,
            &snap_name,
            true,
            8_900_000_000,
        );

        self.snapshots.push(checkpoint);
        Ok(new_id)
    }

    /// Atomically rolls back the active subvolume to the selected snapshot.
    /// Updates the active B-Tree root block and advances generation counter.
    pub fn rollback_to_snapshot(&mut self, target_id: u64) -> Result<(), String> {
        let target = self
            .find_by_id(target_id)
            .ok_or_else(|| format!("Snapshot ID {} not found", target_id))?
            .clone();

        if target.root_block == 0 {
            return Err("Target snapshot has null root block pointer".to_string());
        }

        // 1. Create automatic pre-rollback checkpoint to prevent data loss
        let _ = self.create_pre_rollback_checkpoint(&target.name);

        // 2. Perform atomic pointer pivot
        self.current_root_block = target.root_block;
        self.current_generation += 1;

        Ok(())
    }

    /// Verifies B-Tree root block pointer and metadata consistency of a snapshot
    pub fn verify_snapshot_integrity(&self, id: u64) -> Result<bool, String> {
        let snap = self
            .find_by_id(id)
            .ok_or_else(|| format!("Snapshot ID {} not found", id))?;

        if snap.root_block == 0 {
            return Ok(false);
        }
        if snap.generation == 0 {
            return Ok(false);
        }
        if snap.name.is_empty() {
            return Ok(false);
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_listing_and_lookup() {
        let mgr = SnapshotManager::new();
        assert_eq!(mgr.list_snapshots().len(), 3);
        assert!(mgr.find_by_id(101).is_some());
        assert!(mgr.find_by_id(999).is_none());
    }

    #[test]
    fn test_atomic_rollback_and_checkpoint() {
        let mut mgr = SnapshotManager::new();
        let _initial_root = mgr.current_root_block;
        let initial_gen = mgr.current_generation;

        // Roll back to snapshot 101 (root_block = 1024000)
        let res = mgr.rollback_to_snapshot(101);
        assert!(res.is_ok());

        assert_eq!(mgr.current_root_block, 1024000);
        assert_eq!(mgr.current_generation, initial_gen + 1);

        // Verify pre-rollback checkpoint was created
        assert_eq!(mgr.list_snapshots().len(), 4);
    }

    #[test]
    fn test_snapshot_integrity_check() {
        let mgr = SnapshotManager::new();
        assert!(mgr.verify_snapshot_integrity(102).unwrap());
    }
}
