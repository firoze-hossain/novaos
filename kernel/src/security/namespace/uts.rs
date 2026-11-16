//! kernel/src/security/namespace/uts.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): UTS Isolation Namespace
//!
//! Provides isolated nodename (hostname) and domainname identifiers:
//!   - Independent container hostnames without affecting host or sibling containers.
//!   - Per-namespace sethostname / setdomainname system call handlers.
//!   - POSIX-compliant uname information provider.

#![allow(dead_code)]

pub const ROOT_UTS_NS: u32 = 0;
pub const MAX_UTS_NAMESPACES: usize = 16;
pub const UTS_FIELD_LEN: usize = 32;

pub const EINVAL: isize = -22;
pub const ENOMEM: isize = -12;
pub const EPERM: isize  = -1;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct UtsName {
    pub sysname: [u8; UTS_FIELD_LEN],
    pub nodename: [u8; UTS_FIELD_LEN],
    pub release: [u8; UTS_FIELD_LEN],
    pub version: [u8; UTS_FIELD_LEN],
    pub machine: [u8; UTS_FIELD_LEN],
    pub domainname: [u8; UTS_FIELD_LEN],
}

impl UtsName {
    pub const fn empty() -> Self {
        Self {
            sysname: [0u8; UTS_FIELD_LEN],
            nodename: [0u8; UTS_FIELD_LEN],
            release: [0u8; UTS_FIELD_LEN],
            version: [0u8; UTS_FIELD_LEN],
            machine: [0u8; UTS_FIELD_LEN],
            domainname: [0u8; UTS_FIELD_LEN],
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct UtsNamespace {
    pub id: u32,
    pub name: UtsName,
    pub ref_count: u32,
    pub allocated: bool,
}

impl UtsNamespace {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            name: UtsName::empty(),
            ref_count: 0,
            allocated: false,
        }
    }
}

pub struct UtsNamespaceManager {
    pub namespaces: [UtsNamespace; MAX_UTS_NAMESPACES],
    next_ns_id: u32,
}

impl UtsNamespaceManager {
    pub const fn empty() -> Self {
        Self {
            namespaces: [UtsNamespace::empty(); MAX_UTS_NAMESPACES],
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
        // Initialize Root UTS Namespace
        let root = &mut self.namespaces[0];
        root.id = ROOT_UTS_NS;
        root.ref_count = 1;
        root.allocated = true;

        Self::copy_str(&mut root.name.sysname, b"NovaOS");
        Self::copy_str(&mut root.name.nodename, b"novaos-server");
        Self::copy_str(&mut root.name.release, b"2.0.0-server");
        Self::copy_str(&mut root.name.version, b"#1 SMP Tickless");
        Self::copy_str(&mut root.name.machine, b"x86_64");
        Self::copy_str(&mut root.name.domainname, b"localdomain");
    }

    /// Clone an existing UTS namespace into a new isolated namespace
    pub fn clone_namespace(&mut self, parent_ns_id: u32) -> Result<u32, isize> {
        let parent_idx = self.find_ns_idx(parent_ns_id).ok_or(EINVAL)?;

        let mut free_idx = None;
        for i in 1..MAX_UTS_NAMESPACES {
            if !self.namespaces[i].allocated {
                free_idx = Some(i);
                break;
            }
        }
        let idx = free_idx.ok_or(ENOMEM)?;

        let new_id = self.next_ns_id;
        self.next_ns_id = self.next_ns_id.wrapping_add(1);

        let mut new_ns = self.namespaces[parent_idx];
        new_ns.id = new_id;
        new_ns.ref_count = 1;
        new_ns.allocated = true;

        self.namespaces[idx] = new_ns;
        Ok(new_id)
    }

    /// Update hostname (nodename) in specified UTS namespace
    pub fn sethostname(&mut self, ns_id: u32, name: &[u8]) -> Result<(), isize> {
        if name.is_empty() || name.len() >= UTS_FIELD_LEN {
            return Err(EINVAL);
        }
        let idx = self.find_ns_idx(ns_id).ok_or(EINVAL)?;
        Self::copy_str(&mut self.namespaces[idx].name.nodename, name);
        Ok(())
    }

    /// Update domainname in specified UTS namespace
    pub fn setdomainname(&mut self, ns_id: u32, domain: &[u8]) -> Result<(), isize> {
        if domain.is_empty() || domain.len() >= UTS_FIELD_LEN {
            return Err(EINVAL);
        }
        let idx = self.find_ns_idx(ns_id).ok_or(EINVAL)?;
        Self::copy_str(&mut self.namespaces[idx].name.domainname, domain);
        Ok(())
    }

    /// Query uname info for this UTS namespace
    pub fn get_uname(&self, ns_id: u32, out: &mut UtsName) -> Result<(), isize> {
        let idx = self.find_ns_idx(ns_id).ok_or(EINVAL)?;
        *out = self.namespaces[idx].name;
        Ok(())
    }

    pub fn get_hostname<'a>(&'a self, ns_id: u32) -> Option<&'a [u8]> {
        let idx = self.find_ns_idx(ns_id)?;
        let node = &self.namespaces[idx].name.nodename;
        let mut len = 0;
        while len < node.len() && node[len] != 0 {
            len += 1;
        }
        Some(&node[..len])
    }

    pub fn retain_namespace(&mut self, ns_id: u32) {
        if let Some(idx) = self.find_ns_idx(ns_id) {
            self.namespaces[idx].ref_count = self.namespaces[idx].ref_count.saturating_add(1);
        }
    }

    pub fn release_namespace(&mut self, ns_id: u32) -> bool {
        if ns_id == ROOT_UTS_NS {
            return false;
        }
        if let Some(idx) = self.find_ns_idx(ns_id) {
            self.namespaces[idx].ref_count = self.namespaces[idx].ref_count.saturating_sub(1);
            if self.namespaces[idx].ref_count == 0 {
                self.namespaces[idx] = UtsNamespace::empty();
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

    fn copy_str(dst: &mut [u8; UTS_FIELD_LEN], src: &[u8]) {
        let copy_len = if src.len() < UTS_FIELD_LEN - 1 { src.len() } else { UTS_FIELD_LEN - 1 };
        dst[..copy_len].copy_from_slice(&src[..copy_len]);
        dst[copy_len] = 0; // Null-terminate
    }
}
