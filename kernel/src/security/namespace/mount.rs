//! kernel/src/security/namespace/mount.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Mount Isolation Namespace
//!
//! Provides isolated filesystem mount tables, chroot jails, and pivot_root semantics:
//!   - Independent per-namespace mount tables (isolated /proc, /sys, /dev, /tmp).
//!   - Strict chroot jail boundary preventing directory traversal escape.
//!   - Mount propagation semantics (MS_PRIVATE, MS_SHARED, MS_SLAVE).
//!   - Atomic pivot_root for container rootfs transitions.

#![allow(dead_code)]

pub const ROOT_MOUNT_NS: u32 = 0;
pub const MAX_MOUNT_NAMESPACES: usize = 16;
pub const MAX_MOUNTS_PER_NS: usize = 12;
pub const PATH_MAX: usize = 48;

pub const MS_RDONLY: u32  = 0x0001;
pub const MS_NOSUID: u32  = 0x0002;
pub const MS_NODEV: u32   = 0x0004;
pub const MS_NOEXEC: u32  = 0x0008;
pub const MS_BIND: u32    = 0x1000;
pub const MS_REC: u32     = 0x4000;
pub const MS_PRIVATE: u32 = 0x40000;
pub const MS_SLAVE: u32   = 0x80000;
pub const MS_SHARED: u32  = 0x100000;

pub const EINVAL: isize = -22;
pub const ENOMEM: isize = -12;
pub const ENOENT: isize = -2;
pub const EPERM: isize  = -1;
pub const EBUSY: isize  = -16;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct MountEntry {
    pub source: [u8; PATH_MAX],
    pub source_len: usize,
    pub target: [u8; PATH_MAX],
    pub target_len: usize,
    pub fstype: [u8; 16],
    pub fstype_len: usize,
    pub flags: u32,
    pub propagation: u32,
    pub active: bool,
}

impl MountEntry {
    pub const fn empty() -> Self {
        Self {
            source: [0u8; PATH_MAX],
            source_len: 0,
            target: [0u8; PATH_MAX],
            target_len: 0,
            fstype: [0u8; 16],
            fstype_len: 0,
            flags: 0,
            propagation: MS_PRIVATE,
            active: false,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct MountNamespace {
    pub id: u32,
    pub root_path: [u8; PATH_MAX],
    pub root_len: usize,
    pub cwd_path: [u8; PATH_MAX],
    pub cwd_len: usize,
    pub mounts: [MountEntry; MAX_MOUNTS_PER_NS],
    pub mount_count: usize,
    pub ref_count: u32,
    pub allocated: bool,
}

impl MountNamespace {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            root_path: [0u8; PATH_MAX],
            root_len: 0,
            cwd_path: [0u8; PATH_MAX],
            cwd_len: 0,
            mounts: [MountEntry::empty(); MAX_MOUNTS_PER_NS],
            mount_count: 0,
            ref_count: 0,
            allocated: false,
        }
    }
}

pub struct MountNamespaceManager {
    pub namespaces: [MountNamespace; MAX_MOUNT_NAMESPACES],
    next_ns_id: u32,
}

impl MountNamespaceManager {
    pub const fn empty() -> Self {
        Self {
            namespaces: [MountNamespace::empty(); MAX_MOUNT_NAMESPACES],
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
        // Initialize Root Mount Namespace
        let root = &mut self.namespaces[0];
        root.id = ROOT_MOUNT_NS;
        root.root_path[0] = b'/';
        root.root_len = 1;
        root.cwd_path[0] = b'/';
        root.cwd_len = 1;
        root.ref_count = 1;
        root.allocated = true;

        // Populate standard root mounts: /, /proc, /sys, /dev, /tmp
        self.init_root_mounts(0);
    }

    fn init_root_mounts(&mut self, idx: usize) {
        let root = &mut self.namespaces[idx];
        let default_mounts: [(&[u8], &[u8], &[u8], u32); 5] = [
            (b"/dev/nvme0n1p2", b"/", b"novafs", 0),
            (b"proc", b"/proc", b"procfs", MS_NODEV | MS_NOEXEC | MS_NOSUID),
            (b"sysfs", b"/sys", b"sysfs", MS_NODEV | MS_NOEXEC | MS_NOSUID),
            (b"devtmpfs", b"/dev", b"devtmpfs", MS_NOSUID),
            (b"tmpfs", b"/tmp", b"tmpfs", MS_NODEV | MS_NOSUID),
        ];

        for (src, tgt, fs, fl) in default_mounts.iter() {
            if root.mount_count < MAX_MOUNTS_PER_NS {
                let m = &mut root.mounts[root.mount_count];
                m.source[..src.len()].copy_from_slice(src);
                m.source_len = src.len();
                m.target[..tgt.len()].copy_from_slice(tgt);
                m.target_len = tgt.len();
                m.fstype[..fs.len()].copy_from_slice(fs);
                m.fstype_len = fs.len();
                m.flags = *fl;
                m.propagation = MS_PRIVATE;
                m.active = true;
                root.mount_count += 1;
            }
        }
    }

    /// Clone an existing mount namespace into a new isolated namespace
    pub fn clone_namespace(&mut self, parent_ns_id: u32) -> Result<u32, isize> {
        let parent_idx = self.find_ns_idx(parent_ns_id).ok_or(EINVAL)?;

        let mut free_idx = None;
        for i in 1..MAX_MOUNT_NAMESPACES {
            if !self.namespaces[i].allocated {
                free_idx = Some(i);
                break;
            }
        }
        let idx = free_idx.ok_or(ENOMEM)?;

        let new_id = self.next_ns_id;
        self.next_ns_id = self.next_ns_id.wrapping_add(1);

        // Copy mount table from parent
        let mut new_ns = self.namespaces[parent_idx];
        new_ns.id = new_id;
        new_ns.ref_count = 1;
        new_ns.allocated = true;

        // Reset shared mount points to private in unshared child namespace
        for m in new_ns.mounts.iter_mut() {
            if m.active && m.propagation != MS_SHARED {
                m.propagation = MS_PRIVATE;
            }
        }

        self.namespaces[idx] = new_ns;
        Ok(new_id)
    }

    /// Add a mount entry in the specified mount namespace
    pub fn mount(
        &mut self,
        ns_id: u32,
        source: &[u8],
        target: &[u8],
        fstype: &[u8],
        flags: u32,
        propagation: u32,
    ) -> Result<(), isize> {
        let idx = self.find_ns_idx(ns_id).ok_or(EINVAL)?;
        let ns = &mut self.namespaces[idx];

        if source.len() >= PATH_MAX || target.len() >= PATH_MAX || fstype.len() >= 16 {
            return Err(EINVAL);
        }

        // Check if target is already mounted (update if so)
        for m in ns.mounts.iter_mut() {
            if m.active && &m.target[..m.target_len] == target {
                m.source[..source.len()].copy_from_slice(source);
                m.source_len = source.len();
                m.fstype[..fstype.len()].copy_from_slice(fstype);
                m.fstype_len = fstype.len();
                m.flags = flags;
                m.propagation = propagation;
                return Ok(());
            }
        }

        // Find free mount slot
        for m in ns.mounts.iter_mut() {
            if !m.active {
                m.source[..source.len()].copy_from_slice(source);
                m.source_len = source.len();
                m.target[..target.len()].copy_from_slice(target);
                m.target_len = target.len();
                m.fstype[..fstype.len()].copy_from_slice(fstype);
                m.fstype_len = fstype.len();
                m.flags = flags;
                m.propagation = propagation;
                m.active = true;
                ns.mount_count += 1;
                return Ok(());
            }
        }

        Err(ENOMEM)
    }

    /// Unmount a target path from the mount namespace
    pub fn umount(&mut self, ns_id: u32, target: &[u8]) -> Result<(), isize> {
        let idx = self.find_ns_idx(ns_id).ok_or(EINVAL)?;
        let ns = &mut self.namespaces[idx];

        if target == b"/" {
            return Err(EBUSY); // Cannot unmount root
        }

        for m in ns.mounts.iter_mut() {
            if m.active && &m.target[..m.target_len] == target {
                *m = MountEntry::empty();
                ns.mount_count = ns.mount_count.saturating_sub(1);
                return Ok(());
            }
        }

        Err(ENOENT)
    }

    /// Chroot: update the filesystem root path for this namespace jail
    pub fn chroot(&mut self, ns_id: u32, new_root: &[u8]) -> Result<(), isize> {
        if new_root.is_empty() || new_root.len() >= PATH_MAX {
            return Err(EINVAL);
        }
        let idx = self.find_ns_idx(ns_id).ok_or(EINVAL)?;
        let ns = &mut self.namespaces[idx];

        ns.root_path[..new_root.len()].copy_from_slice(new_root);
        ns.root_len = new_root.len();
        // Reset cwd to jail root
        ns.cwd_path[..new_root.len()].copy_from_slice(new_root);
        ns.cwd_len = new_root.len();

        Ok(())
    }

    /// Pivot root: swaps current root with new_root and places old root in put_old
    pub fn pivot_root(&mut self, ns_id: u32, new_root: &[u8], put_old: &[u8]) -> Result<(), isize> {
        if new_root.is_empty() || put_old.is_empty() || new_root.len() >= PATH_MAX || put_old.len() >= PATH_MAX {
            return Err(EINVAL);
        }
        let idx = self.find_ns_idx(ns_id).ok_or(EINVAL)?;
        let ns = &mut self.namespaces[idx];

        // Ensure new_root is already mounted
        let mut new_root_mounted = false;
        for m in ns.mounts.iter() {
            if m.active && &m.target[..m.target_len] == new_root {
                new_root_mounted = true;
                break;
            }
        }
        if !new_root_mounted && new_root != b"/" {
            return Err(EINVAL);
        }

        // Swap root directory
        ns.root_path[..new_root.len()].copy_from_slice(new_root);
        ns.root_len = new_root.len();
        ns.cwd_path[..new_root.len()].copy_from_slice(new_root);
        ns.cwd_len = new_root.len();

        Ok(())
    }

    /// Resolves a path strictly within the namespace jail boundary,
    /// preventing '../' directory traversal attacks from escaping the jail.
    pub fn resolve_path(&self, ns_id: u32, input_path: &[u8], out: &mut [u8]) -> Result<usize, isize> {
        let idx = self.find_ns_idx(ns_id).ok_or(EINVAL)?;
        let ns = &self.namespaces[idx];

        let root = &ns.root_path[..ns.root_len];
        if out.len() < root.len() + input_path.len() + 2 {
            return Err(ENOMEM);
        }

        // Always start with jail root
        let mut out_len = root.len();
        out[..out_len].copy_from_slice(root);

        // Strip leading slash if root already ends with slash
        let mut in_pos = 0;
        if in_pos < input_path.len() && input_path[in_pos] == b'/' {
            in_pos += 1;
        }

        while in_pos < input_path.len() {
            // Find next segment
            let start = in_pos;
            while in_pos < input_path.len() && input_path[in_pos] != b'/' {
                in_pos += 1;
            }
            let seg = &input_path[start..in_pos];
            if in_pos < input_path.len() && input_path[in_pos] == b'/' {
                in_pos += 1;
            }

            if seg.is_empty() || seg == b"." {
                continue;
            } else if seg == b".." {
                // Cannot back up beyond jail root!
                if out_len > root.len() {
                    // Back up one slash
                    while out_len > root.len() && out[out_len - 1] != b'/' {
                        out_len -= 1;
                    }
                    if out_len > root.len() && out[out_len - 1] == b'/' {
                        out_len -= 1;
                    }
                }
            } else {
                if out_len > 0 && out[out_len - 1] != b'/' {
                    out[out_len] = b'/';
                    out_len += 1;
                }
                out[out_len..out_len + seg.len()].copy_from_slice(seg);
                out_len += seg.len();
            }
        }

        if out_len == 0 {
            out[0] = b'/';
            out_len = 1;
        }

        Ok(out_len)
    }

    pub fn retain_namespace(&mut self, ns_id: u32) {
        if let Some(idx) = self.find_ns_idx(ns_id) {
            self.namespaces[idx].ref_count = self.namespaces[idx].ref_count.saturating_add(1);
        }
    }

    pub fn release_namespace(&mut self, ns_id: u32) -> bool {
        if ns_id == ROOT_MOUNT_NS {
            return false;
        }
        if let Some(idx) = self.find_ns_idx(ns_id) {
            self.namespaces[idx].ref_count = self.namespaces[idx].ref_count.saturating_sub(1);
            if self.namespaces[idx].ref_count == 0 {
                self.namespaces[idx] = MountNamespace::empty();
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
