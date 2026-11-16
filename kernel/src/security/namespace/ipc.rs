//! kernel/src/security/namespace/ipc.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): IPC Isolation Namespace
//!
//! Provides isolated System V and POSIX IPC resources:
//!   - Partitioned shared memory (shm) segments.
//!   - Partitioned message queues (mq).
//!   - Partitioned semaphores (sem).
//!   - Complete isolation of IPC keys and identifiers between containers.

#![allow(dead_code)]

pub const ROOT_IPC_NS: u32 = 0;
pub const MAX_IPC_NAMESPACES: usize = 16;
pub const MAX_SHM_PER_NS: usize = 8;
pub const MAX_MQ_PER_NS: usize = 8;
pub const MAX_SEM_PER_NS: usize = 8;

pub const IPC_CREAT: u32  = 0o1000;
pub const IPC_EXCL: u32   = 0o2000;
pub const IPC_RMID: u32   = 0;

pub const EINVAL: isize = -22;
pub const ENOMEM: isize = -12;
pub const EEXIST: isize = -17;
pub const ENOENT: isize = -2;
pub const EACCES: isize = -13;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct IpcShmSegment {
    pub shm_id: u32,
    pub key: u32,
    pub size: usize,
    pub owner_uid: u32,
    pub perms: u16,
    pub attach_count: u32,
    pub active: bool,
}

impl IpcShmSegment {
    pub const fn empty() -> Self {
        Self {
            shm_id: 0,
            key: 0,
            size: 0,
            owner_uid: 0,
            perms: 0,
            attach_count: 0,
            active: false,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct IpcMsgQueue {
    pub mq_id: u32,
    pub key: u32,
    pub owner_uid: u32,
    pub max_msgs: u32,
    pub msg_count: u32,
    pub active: bool,
}

impl IpcMsgQueue {
    pub const fn empty() -> Self {
        Self {
            mq_id: 0,
            key: 0,
            owner_uid: 0,
            max_msgs: 0,
            msg_count: 0,
            active: false,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct IpcSemaphore {
    pub sem_id: u32,
    pub key: u32,
    pub val: i32,
    pub active: bool,
}

impl IpcSemaphore {
    pub const fn empty() -> Self {
        Self {
            sem_id: 0,
            key: 0,
            val: 0,
            active: false,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct IpcNamespace {
    pub id: u32,
    pub shm: [IpcShmSegment; MAX_SHM_PER_NS],
    pub mq: [IpcMsgQueue; MAX_MQ_PER_NS],
    pub sem: [IpcSemaphore; MAX_SEM_PER_NS],
    pub next_obj_id: u32,
    pub ref_count: u32,
    pub allocated: bool,
}

impl IpcNamespace {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            shm: [IpcShmSegment::empty(); MAX_SHM_PER_NS],
            mq: [IpcMsgQueue::empty(); MAX_MQ_PER_NS],
            sem: [IpcSemaphore::empty(); MAX_SEM_PER_NS],
            next_obj_id: 1,
            ref_count: 0,
            allocated: false,
        }
    }
}

pub struct IpcNamespaceManager {
    pub namespaces: [IpcNamespace; MAX_IPC_NAMESPACES],
    next_ns_id: u32,
}

impl IpcNamespaceManager {
    pub const fn empty() -> Self {
        Self {
            namespaces: [IpcNamespace::empty(); MAX_IPC_NAMESPACES],
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
        // Root IPC namespace
        let root = &mut self.namespaces[0];
        root.id = ROOT_IPC_NS;
        root.ref_count = 1;
        root.allocated = true;
    }

    pub fn create_namespace(&mut self) -> Result<u32, isize> {
        let mut free_idx = None;
        for i in 1..MAX_IPC_NAMESPACES {
            if !self.namespaces[i].allocated {
                free_idx = Some(i);
                break;
            }
        }
        let idx = free_idx.ok_or(ENOMEM)?;

        let new_id = self.next_ns_id;
        self.next_ns_id = self.next_ns_id.wrapping_add(1);

        let mut new_ns = IpcNamespace::empty();
        new_ns.id = new_id;
        new_ns.ref_count = 1;
        new_ns.allocated = true;

        self.namespaces[idx] = new_ns;
        Ok(new_id)
    }

    /// Obtain or create a shared memory segment in this IPC namespace
    pub fn shm_get(
        &mut self,
        ns_id: u32,
        key: u32,
        size: usize,
        flags: u32,
        owner_uid: u32,
    ) -> Result<u32, isize> {
        let idx = self.find_ns_idx(ns_id).ok_or(EINVAL)?;
        let ns = &mut self.namespaces[idx];

        // Search for existing key
        for seg in ns.shm.iter() {
            if seg.active && seg.key == key {
                if (flags & IPC_CREAT != 0) && (flags & IPC_EXCL != 0) {
                    return Err(EEXIST);
                }
                return Ok(seg.shm_id);
            }
        }

        if (flags & IPC_CREAT) == 0 {
            return Err(ENOENT);
        }

        // Allocate new segment
        for seg in ns.shm.iter_mut() {
            if !seg.active {
                let shm_id = ns.next_obj_id;
                ns.next_obj_id = ns.next_obj_id.wrapping_add(1);

                *seg = IpcShmSegment {
                    shm_id,
                    key,
                    size,
                    owner_uid,
                    perms: (flags & 0o777) as u16,
                    attach_count: 0,
                    active: true,
                };
                return Ok(shm_id);
            }
        }

        Err(ENOMEM)
    }

    /// Obtain or create a message queue in this IPC namespace
    pub fn mq_open(&mut self, ns_id: u32, key: u32, max_msgs: u32, owner_uid: u32) -> Result<u32, isize> {
        let idx = self.find_ns_idx(ns_id).ok_or(EINVAL)?;
        let ns = &mut self.namespaces[idx];

        for q in ns.mq.iter() {
            if q.active && q.key == key {
                return Ok(q.mq_id);
            }
        }

        for q in ns.mq.iter_mut() {
            if !q.active {
                let mq_id = ns.next_obj_id;
                ns.next_obj_id = ns.next_obj_id.wrapping_add(1);

                *q = IpcMsgQueue {
                    mq_id,
                    key,
                    owner_uid,
                    max_msgs,
                    msg_count: 0,
                    active: true,
                };
                return Ok(mq_id);
            }
        }

        Err(ENOMEM)
    }

    pub fn retain_namespace(&mut self, ns_id: u32) {
        if let Some(idx) = self.find_ns_idx(ns_id) {
            self.namespaces[idx].ref_count = self.namespaces[idx].ref_count.saturating_add(1);
        }
    }

    pub fn release_namespace(&mut self, ns_id: u32) -> bool {
        if ns_id == ROOT_IPC_NS {
            return false;
        }
        if let Some(idx) = self.find_ns_idx(ns_id) {
            self.namespaces[idx].ref_count = self.namespaces[idx].ref_count.saturating_sub(1);
            if self.namespaces[idx].ref_count == 0 {
                self.namespaces[idx] = IpcNamespace::empty();
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
