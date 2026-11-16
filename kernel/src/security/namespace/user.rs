//! kernel/src/security/namespace/user.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): User Isolation Namespace
//!
//! Provides UID / GID mapping and scoped privilege separation:
//!   - Unprivileged host users can map to root (UID 0) inside container namespaces.
//!   - Bidirectional UID/GID translation between container and host credentials.
//!   - Confined container root capabilities (CAP_CHROOT, CAP_NET_ADMIN, CAP_SYS_ADMIN)
//!     without leaking root privileges to host resources.

#![allow(dead_code)]

pub const ROOT_USER_NS: u32 = 0;
pub const MAX_USER_NAMESPACES: usize = 16;
pub const MAX_ID_MAP_ENTRIES: usize = 4;

pub const CAP_CHROOT: u32       = 1 << 0;
pub const CAP_NET_ADMIN: u32    = 1 << 1;
pub const CAP_SYS_ADMIN: u32    = 1 << 2;
pub const CAP_SETUID: u32       = 1 << 3;
pub const CAP_SETGID: u32       = 1 << 4;
pub const CAP_DAC_OVERRIDE: u32 = 1 << 5;
pub const CAP_ALL: u32          = 0x003F;

pub const EINVAL: isize = -22;
pub const ENOMEM: isize = -12;
pub const EPERM: isize  = -1;
pub const EEXIST: isize = -17;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct IdMapEntry {
    pub inside_id: u32,
    pub outside_id: u32,
    pub count: u32,
    pub active: bool,
}

impl IdMapEntry {
    pub const fn empty() -> Self {
        Self {
            inside_id: 0,
            outside_id: 0,
            count: 0,
            active: false,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct UserNamespace {
    pub id: u32,
    pub parent_id: Option<u32>,
    pub owner_uid: u32,
    pub uid_maps: [IdMapEntry; MAX_ID_MAP_ENTRIES],
    pub uid_map_count: usize,
    pub gid_maps: [IdMapEntry; MAX_ID_MAP_ENTRIES],
    pub gid_map_count: usize,
    pub capabilities: u32,
    pub ref_count: u32,
    pub allocated: bool,
}

impl UserNamespace {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            parent_id: None,
            owner_uid: 0,
            uid_maps: [IdMapEntry::empty(); MAX_ID_MAP_ENTRIES],
            uid_map_count: 0,
            gid_maps: [IdMapEntry::empty(); MAX_ID_MAP_ENTRIES],
            gid_map_count: 0,
            capabilities: 0,
            ref_count: 0,
            allocated: false,
        }
    }
}

pub struct UserNamespaceManager {
    pub namespaces: [UserNamespace; MAX_USER_NAMESPACES],
    next_ns_id: u32,
}

impl UserNamespaceManager {
    pub const fn empty() -> Self {
        Self {
            namespaces: [UserNamespace::empty(); MAX_USER_NAMESPACES],
            next_ns_id: 1,
        }
    }

    pub fn new() -> Self {
        let mut mgr = Self::empty();
        mgr.init();
        mgr
    }

    pub fn init(&mut self) {
        *self = Self::empty();
        // Initialize Root User Namespace
        let root = &mut self.namespaces[0];
        root.id = ROOT_USER_NS;
        root.owner_uid = 0;
        root.capabilities = CAP_ALL;
        root.ref_count = 1;
        root.allocated = true;

        // Identity mapping for root namespace (0 -> 0 up to 65536)
        root.uid_maps[0] = IdMapEntry {
            inside_id: 0,
            outside_id: 0,
            count: 65536,
            active: true,
        };
        root.uid_map_count = 1;

        root.gid_maps[0] = IdMapEntry {
            inside_id: 0,
            outside_id: 0,
            count: 65536,
            active: true,
        };
        root.gid_map_count = 1;
    }

    /// Create a new user namespace owned by `owner_uid`
    pub fn create_namespace(&mut self, owner_uid: u32, parent_ns_id: u32) -> Result<u32, isize> {
        let parent_idx = self.find_ns_idx(parent_ns_id).ok_or(EINVAL)?;

        let mut free_idx = None;
        for i in 1..MAX_USER_NAMESPACES {
            if !self.namespaces[i].allocated {
                free_idx = Some(i);
                break;
            }
        }
        let idx = free_idx.ok_or(ENOMEM)?;

        let new_id = self.next_ns_id;
        self.next_ns_id = self.next_ns_id.wrapping_add(1);

        let mut new_ns = UserNamespace::empty();
        new_ns.id = new_id;
        new_ns.parent_id = Some(parent_ns_id);
        new_ns.owner_uid = owner_uid;
        // Inside this new user namespace, creator possesses all container capabilities
        new_ns.capabilities = CAP_ALL;
        new_ns.ref_count = 1;
        new_ns.allocated = true;

        self.namespaces[parent_idx].ref_count = self.namespaces[parent_idx].ref_count.saturating_add(1);
        self.namespaces[idx] = new_ns;

        Ok(new_id)
    }

    /// Add a UID mapping entry: inside_id <-> outside_id (length `count`)
    pub fn set_uid_map(
        &mut self,
        ns_id: u32,
        inside_id: u32,
        outside_id: u32,
        count: u32,
    ) -> Result<(), isize> {
        if count == 0 {
            return Err(EINVAL);
        }
        let idx = self.find_ns_idx(ns_id).ok_or(EINVAL)?;
        let ns = &mut self.namespaces[idx];

        if ns.uid_map_count >= MAX_ID_MAP_ENTRIES {
            return Err(ENOMEM);
        }

        // Check for range overlap
        for m in ns.uid_maps.iter() {
            if m.active {
                if (inside_id < m.inside_id + m.count) && (inside_id + count > m.inside_id) {
                    return Err(EINVAL);
                }
            }
        }

        for m in ns.uid_maps.iter_mut() {
            if !m.active {
                *m = IdMapEntry {
                    inside_id,
                    outside_id,
                    count,
                    active: true,
                };
                ns.uid_map_count += 1;
                return Ok(());
            }
        }

        Err(ENOMEM)
    }

    /// Add a GID mapping entry: inside_id <-> outside_id (length `count`)
    pub fn set_gid_map(
        &mut self,
        ns_id: u32,
        inside_id: u32,
        outside_id: u32,
        count: u32,
    ) -> Result<(), isize> {
        if count == 0 {
            return Err(EINVAL);
        }
        let idx = self.find_ns_idx(ns_id).ok_or(EINVAL)?;
        let ns = &mut self.namespaces[idx];

        if ns.gid_map_count >= MAX_ID_MAP_ENTRIES {
            return Err(ENOMEM);
        }

        for m in ns.gid_maps.iter() {
            if m.active {
                if (inside_id < m.inside_id + m.count) && (inside_id + count > m.inside_id) {
                    return Err(EINVAL);
                }
            }
        }

        for m in ns.gid_maps.iter_mut() {
            if !m.active {
                *m = IdMapEntry {
                    inside_id,
                    outside_id,
                    count,
                    active: true,
                };
                ns.gid_map_count += 1;
                return Ok(());
            }
        }

        Err(ENOMEM)
    }

    /// Translate container-internal UID to host-external UID
    pub fn map_uid_to_outside(&self, ns_id: u32, inside_uid: u32) -> Option<u32> {
        let idx = self.find_ns_idx(ns_id)?;
        let ns = &self.namespaces[idx];

        for m in ns.uid_maps.iter() {
            if m.active && inside_uid >= m.inside_id && inside_uid < (m.inside_id + m.count) {
                let offset = inside_uid - m.inside_id;
                return Some(m.outside_id + offset);
            }
        }
        None
    }

    /// Translate host-external UID to container-internal UID
    pub fn map_uid_to_inside(&self, ns_id: u32, outside_uid: u32) -> Option<u32> {
        let idx = self.find_ns_idx(ns_id)?;
        let ns = &self.namespaces[idx];

        for m in ns.uid_maps.iter() {
            if m.active && outside_uid >= m.outside_id && outside_uid < (m.outside_id + m.count) {
                let offset = outside_uid - m.outside_id;
                return Some(m.inside_id + offset);
            }
        }
        None
    }

    /// Check if a process with `inside_uid` in `ns_id` holds capability `cap`
    pub fn has_capability(&self, ns_id: u32, inside_uid: u32, cap: u32) -> bool {
        // UID 0 inside the user namespace has full container capabilities
        if inside_uid == 0 {
            if let Some(idx) = self.find_ns_idx(ns_id) {
                return (self.namespaces[idx].capabilities & cap) == cap;
            }
        }
        // Host root has capabilities everywhere
        if let Some(outside_uid) = self.map_uid_to_outside(ns_id, inside_uid) {
            if outside_uid == 0 {
                return true;
            }
        }
        false
    }

    pub fn retain_namespace(&mut self, ns_id: u32) {
        if let Some(idx) = self.find_ns_idx(ns_id) {
            self.namespaces[idx].ref_count = self.namespaces[idx].ref_count.saturating_add(1);
        }
    }

    pub fn release_namespace(&mut self, ns_id: u32) -> bool {
        if ns_id == ROOT_USER_NS {
            return false;
        }
        if let Some(idx) = self.find_ns_idx(ns_id) {
            self.namespaces[idx].ref_count = self.namespaces[idx].ref_count.saturating_sub(1);
            if self.namespaces[idx].ref_count == 0 {
                let parent = self.namespaces[idx].parent_id;
                self.namespaces[idx] = UserNamespace::empty();
                if let Some(p) = parent {
                    self.release_namespace(p);
                }
                return true;
            }
        }
        false
    }

    pub fn find_ns_idx(&self, ns_id: u32) -> Option<usize> {
        for (i, ns) in self.namespaces.iter().enumerate() {
            if ns.allocated && ns.id == ns_id {
                return Some(i);
            }
        }
        None
    }
}
