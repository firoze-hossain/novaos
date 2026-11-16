//! kernel/src/security/namespace/pid.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): PID Isolation Namespace
//!
//! Provides independent process ID numbering spaces and hierarchical process trees:
//!   - Nested multi-level PID namespaces (up to 16 levels of depth).
//!   - Dedicated PID 1 (container init) per namespace with cascading kill on init exit.
//!   - Bidirectional process ID translation: (ns_id, local_pid) <-> host_pid.
//!   - Strict visibility boundary: child namespaces cannot see or signal parent/sibling processes.

#![allow(dead_code)]

pub const ROOT_PID_NS: u32 = 0;
pub const MAX_PID_NAMESPACES: usize = 16;
pub const MAX_PID_MAPPINGS: usize = 64;
pub const MAX_PID_DEPTH: usize = 16;

pub const EINVAL: isize = -22;
pub const ENOMEM: isize = -12;
pub const ESRCH: isize = -3;
pub const EEXIST: isize = -17;
pub const EPERM: isize = -1;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PidNamespace {
    pub id: u32,
    pub parent_id: Option<u32>,
    pub depth: u32,
    pub init_pid: Option<u32>,      // Host PID acting as PID 1 in this namespace
    pub next_local_pid: u32,
    pub ref_count: u32,
    pub active_processes: u32,
    pub alive: bool,
    pub allocated: bool,
}

impl PidNamespace {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            parent_id: None,
            depth: 0,
            init_pid: None,
            next_local_pid: 1,
            ref_count: 0,
            active_processes: 0,
            alive: false,
            allocated: false,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PidMapping {
    pub host_pid: u32,
    pub ns_id: u32,
    pub local_pid: u32,
    pub active: bool,
}

impl PidMapping {
    pub const fn empty() -> Self {
        Self {
            host_pid: 0,
            ns_id: 0,
            local_pid: 0,
            active: false,
        }
    }
}

pub struct PidNamespaceManager {
    pub namespaces: [PidNamespace; MAX_PID_NAMESPACES],
    pub mappings: [PidMapping; MAX_PID_MAPPINGS],
    next_ns_id: u32,
}

impl PidNamespaceManager {
    pub const fn empty() -> Self {
        Self {
            namespaces: [PidNamespace::empty(); MAX_PID_NAMESPACES],
            mappings: [PidMapping::empty(); MAX_PID_MAPPINGS],
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
        // Initialize Root PID Namespace (NS 0)
        self.namespaces[0] = PidNamespace {
            id: ROOT_PID_NS,
            parent_id: None,
            depth: 0,
            init_pid: Some(1),
            next_local_pid: 2, // PID 1 is root init
            ref_count: 1,
            active_processes: 1,
            alive: true,
            allocated: true,
        };

        // Root mapping for host PID 1
        self.mappings[0] = PidMapping {
            host_pid: 1,
            ns_id: ROOT_PID_NS,
            local_pid: 1,
            active: true,
        };
    }

    /// Create a new child PID namespace under `parent_ns_id`
    pub fn create_namespace(&mut self, parent_ns_id: u32) -> Result<u32, isize> {
        let parent_idx = self.find_ns_idx(parent_ns_id).ok_or(EINVAL)?;
        let parent_depth = self.namespaces[parent_idx].depth;
        if parent_depth >= (MAX_PID_DEPTH as u32) {
            return Err(EINVAL); // Nesting limit exceeded
        }

        // Find free namespace slot
        let mut free_idx = None;
        for i in 1..MAX_PID_NAMESPACES {
            if !self.namespaces[i].allocated {
                free_idx = Some(i);
                break;
            }
        }
        let idx = free_idx.ok_or(ENOMEM)?;

        let new_id = self.next_ns_id;
        self.next_ns_id = self.next_ns_id.wrapping_add(1);

        self.namespaces[idx] = PidNamespace {
            id: new_id,
            parent_id: Some(parent_ns_id),
            depth: parent_depth + 1,
            init_pid: None, // Will be set on first process registration
            next_local_pid: 1,
            ref_count: 1,
            active_processes: 0,
            alive: true,
            allocated: true,
        };

        // Increment parent reference count
        self.namespaces[parent_idx].ref_count = self.namespaces[parent_idx].ref_count.saturating_add(1);

        Ok(new_id)
    }

    /// Register a newly spawned process (`host_pid`) into `target_ns_id`
    /// Returns the assigned `local_pid` in `target_ns_id`.
    pub fn register_process(&mut self, host_pid: u32, target_ns_id: u32) -> Result<u32, isize> {
        let ns_idx = self.find_ns_idx(target_ns_id).ok_or(EINVAL)?;
        if !self.namespaces[ns_idx].alive {
            return Err(ESRCH); // Namespace is dead (init died)
        }

        let local_pid = self.namespaces[ns_idx].next_local_pid;
        self.namespaces[ns_idx].next_local_pid = self.namespaces[ns_idx].next_local_pid.saturating_add(1);
        self.namespaces[ns_idx].active_processes = self.namespaces[ns_idx].active_processes.saturating_add(1);

        if self.namespaces[ns_idx].init_pid.is_none() {
            self.namespaces[ns_idx].init_pid = Some(host_pid);
        }

        // Add mapping for this namespace
        self.add_mapping(host_pid, target_ns_id, local_pid)?;

        // Also add mappings up the ancestor chain if this is a nested namespace
        let mut current_ns = self.namespaces[ns_idx].parent_id;
        while let Some(ancestor_id) = current_ns {
            if let Some(anc_idx) = self.find_ns_idx(ancestor_id) {
                // If not already mapped in ancestor
                if self.translate_to_local(host_pid, ancestor_id).is_none() {
                    let anc_local_pid = self.namespaces[anc_idx].next_local_pid;
                    self.namespaces[anc_idx].next_local_pid = anc_local_pid.saturating_add(1);
                    self.namespaces[anc_idx].active_processes = self.namespaces[anc_idx].active_processes.saturating_add(1);
                    let _ = self.add_mapping(host_pid, ancestor_id, anc_local_pid);
                }
                current_ns = self.namespaces[anc_idx].parent_id;
            } else {
                break;
            }
        }

        Ok(local_pid)
    }

    /// Translate a global host PID to the local PID visible inside `target_ns_id`
    pub fn translate_to_local(&self, host_pid: u32, target_ns_id: u32) -> Option<u32> {
        for m in self.mappings.iter() {
            if m.active && m.host_pid == host_pid && m.ns_id == target_ns_id {
                return Some(m.local_pid);
            }
        }
        None
    }

    /// Translate a namespace-local PID back to its global host PID
    pub fn translate_to_host(&self, local_pid: u32, ns_id: u32) -> Option<u32> {
        for m in self.mappings.iter() {
            if m.active && m.local_pid == local_pid && m.ns_id == ns_id {
                return Some(m.host_pid);
            }
        }
        None
    }

    /// Check if a process in `caller_ns_id` is allowed to see/signal `target_host_pid`
    pub fn can_see_process(&self, caller_ns_id: u32, target_host_pid: u32) -> bool {
        // A process can only see target if target has a valid mapping in caller's namespace
        self.translate_to_local(target_host_pid, caller_ns_id).is_some()
    }

    /// Check if `child_ns_id` is equal to or a descendant of `ancestor_ns_id`
    pub fn is_descendant(&self, child_ns_id: u32, ancestor_ns_id: u32) -> bool {
        if child_ns_id == ancestor_ns_id {
            return true;
        }
        let mut curr = child_ns_id;
        while let Some(idx) = self.find_ns_idx(curr) {
            match self.namespaces[idx].parent_id {
                Some(parent) => {
                    if parent == ancestor_ns_id {
                        return true;
                    }
                    curr = parent;
                }
                None => break,
            }
        }
        false
    }

    /// Unregister a process on termination. Returns `Some(ns_id)` if this was
    /// the init process of a namespace, signaling cascading termination.
    pub fn unregister_process(&mut self, host_pid: u32) -> Option<u32> {
        let mut killed_ns_init = None;

        for i in 0..MAX_PID_MAPPINGS {
            if self.mappings[i].active && self.mappings[i].host_pid == host_pid {
                self.mappings[i].active = false;
                let ns_id = self.mappings[i].ns_id;
                if let Some(idx) = self.find_ns_idx(ns_id) {
                    self.namespaces[idx].active_processes = self.namespaces[idx].active_processes.saturating_sub(1);
                    // Check if host_pid was init of this namespace
                    if self.namespaces[idx].init_pid == Some(host_pid) && ns_id != ROOT_PID_NS {
                        self.namespaces[idx].alive = false;
                        killed_ns_init = Some(ns_id);
                    }
                }
            }
        }

        killed_ns_init
    }

    /// Increments reference count for namespace
    pub fn retain_namespace(&mut self, ns_id: u32) {
        if let Some(idx) = self.find_ns_idx(ns_id) {
            self.namespaces[idx].ref_count = self.namespaces[idx].ref_count.saturating_add(1);
        }
    }

    /// Decrements reference count, frees namespace if 0
    pub fn release_namespace(&mut self, ns_id: u32) -> bool {
        if ns_id == ROOT_PID_NS {
            return false; // Root namespace is permanent
        }
        if let Some(idx) = self.find_ns_idx(ns_id) {
            self.namespaces[idx].ref_count = self.namespaces[idx].ref_count.saturating_sub(1);
            if self.namespaces[idx].ref_count == 0 {
                let parent = self.namespaces[idx].parent_id;
                self.namespaces[idx] = PidNamespace::empty();
                // Release parent reference
                if let Some(p) = parent {
                    self.release_namespace(p);
                }
                return true;
            }
        }
        false
    }

    pub fn is_namespace_alive(&self, ns_id: u32) -> bool {
        if let Some(idx) = self.find_ns_idx(ns_id) {
            self.namespaces[idx].alive
        } else {
            false
        }
    }

    pub fn active_namespaces_count(&self) -> usize {
        self.namespaces.iter().filter(|ns| ns.allocated).count()
    }

    pub fn find_ns_idx(&self, ns_id: u32) -> Option<usize> {
        for (i, ns) in self.namespaces.iter().enumerate() {
            if ns.allocated && ns.id == ns_id {
                return Some(i);
            }
        }
        None
    }

    fn add_mapping(&mut self, host_pid: u32, ns_id: u32, local_pid: u32) -> Result<(), isize> {
        for m in self.mappings.iter_mut() {
            if !m.active {
                *m = PidMapping {
                    host_pid,
                    ns_id,
                    local_pid,
                    active: true,
                };
                return Ok(());
            }
        }
        Err(ENOMEM)
    }
}
