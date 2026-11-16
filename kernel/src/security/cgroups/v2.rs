//! kernel/src/security/cgroups/v2.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Unified Hierarchy Control Groups v2
//!
//! Implements comprehensive Linux-grade cgroups v2 resource governance:
//!   - Single unified hierarchy rooted at /sys/fs/cgroup.
//!   - Subtree control enabling/disabling controllers per level.
//!   - Memory controller: memory.max hard limits with OOM killer, memory.high soft
//!     throttle, memory.current, memory.stat, memory.oom.group.
//!   - CPU controller: CFS bandwidth quotas (cpu.max quota/period), cpu.weight, cpu.stat.
//!   - I/O controller: per-device rate limiting (io.max rbps/wbps/riops/wiops), io.weight.
//!   - PIDs controller: process count limits (pids.max, pids.current, pids.events).
//!   - Cgroup freezing (cgroup.freeze, cgroup.events).

#![allow(dead_code)]

pub use super::memory::MemoryControllerState;
pub use super::cpu::CpuControllerState;
pub use super::io::IoControllerState;
pub use super::pids::PidsControllerState;

use super::memory::{append_kv, copy_slice, format_u64_line, EINVAL, ENOMEM};

pub const ROOT_CGROUP_ID: u32 = 0;
pub const MAX_CGROUPS: usize = 32;
pub const MAX_PROCS_PER_CGROUP: usize = 64;
pub const CGROUP_NAME_MAX: usize = 32;
pub const CGROUP_PATH_MAX: usize = 96;

pub const CONTROLLER_CPU: u32    = 1 << 0;
pub const CONTROLLER_MEMORY: u32 = 1 << 1;
pub const CONTROLLER_IO: u32     = 1 << 2;
pub const CONTROLLER_PIDS: u32   = 1 << 3;
pub const CONTROLLER_ALL: u32    = CONTROLLER_CPU | CONTROLLER_MEMORY | CONTROLLER_IO | CONTROLLER_PIDS;

pub const CGROUP_TYPE_DOMAIN: u8 = 0;
pub const CGROUP_TYPE_THREADED: u8 = 1;

pub const ENOENT: isize = -2;
pub const ESRCH: isize  = -3;
pub const EEXIST: isize = -17;
pub const EBUSY: isize  = -16;
pub const EPERM: isize  = -1;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CgroupNode {
    pub id: u32,
    pub parent_id: Option<u32>,
    pub name: [u8; CGROUP_NAME_MAX],
    pub name_len: u8,
    pub path: [u8; CGROUP_PATH_MAX],
    pub path_len: u8,
    pub cgroup_type: u8,
    pub controllers_available: u32,
    pub subtree_control: u32,
    pub frozen: bool,
    pub procs: [u32; MAX_PROCS_PER_CGROUP],
    pub proc_count: usize,
    pub memory: MemoryControllerState,
    pub cpu: CpuControllerState,
    pub io: IoControllerState,
    pub pids: PidsControllerState,
    pub active: bool,
}

impl CgroupNode {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            parent_id: None,
            name: [0; CGROUP_NAME_MAX],
            name_len: 0,
            path: [0; CGROUP_PATH_MAX],
            path_len: 0,
            cgroup_type: CGROUP_TYPE_DOMAIN,
            controllers_available: 0,
            subtree_control: 0,
            frozen: false,
            procs: [0; MAX_PROCS_PER_CGROUP],
            proc_count: 0,
            memory: MemoryControllerState::empty(),
            cpu: CpuControllerState::empty(),
            io: IoControllerState::empty(),
            pids: PidsControllerState::empty(),
            active: false,
        }
    }

    pub fn name_as_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("")
    }

    pub fn path_as_str(&self) -> &str {
        core::str::from_utf8(&self.path[..self.path_len as usize]).unwrap_or("")
    }
}

pub struct CgroupManager {
    pub cgroups: [CgroupNode; MAX_CGROUPS],
    pub next_id: u32,
    pub proc_map: [Option<u32>; 256],
    pub proc_memory: [u64; 256],
    pub initialized: bool,
}

impl CgroupManager {
    pub const fn empty() -> Self {
        Self {
            cgroups: [CgroupNode::empty(); MAX_CGROUPS],
            next_id: 1,
            proc_map: [None; 256],
            proc_memory: [0; 256],
            initialized: false,
        }
    }

    pub fn new() -> Self {
        let mut mgr = Self::empty();
        mgr.init();
        mgr
    }

    pub fn reset(&mut self) {
        *self = Self::empty();
        self.init();
    }

    /// Initialize Root Cgroup (id 0) at path "/"
    pub fn init(&mut self) {
        *self = Self::empty();

        let root = &mut self.cgroups[0];
        root.id = ROOT_CGROUP_ID;
        root.parent_id = None;
        root.name[0] = b'/';
        root.name_len = 1;
        root.path[0] = b'/';
        root.path_len = 1;
        root.cgroup_type = CGROUP_TYPE_DOMAIN;
        root.controllers_available = CONTROLLER_ALL;
        root.subtree_control = CONTROLLER_ALL;
        root.frozen = false;
        root.memory = MemoryControllerState::empty();
        root.cpu = CpuControllerState::empty();
        root.io = IoControllerState::empty();
        root.pids = PidsControllerState::empty();
        root.active = true;

        // Attach Init Process (PID 1) to Root Cgroup
        root.procs[0] = 1;
        root.proc_count = 1;
        root.pids.current_pids = 1;
        self.proc_map[1] = Some(ROOT_CGROUP_ID);

        self.next_id = 1;
        self.initialized = true;
    }

    /// Create a new child cgroup under `parent_id` with directory name `name`.
    pub fn create_cgroup(&mut self, parent_id: u32, name: &str) -> Result<u32, isize> {
        let name_bytes = name.as_bytes();
        if name_bytes.is_empty() || name_bytes.len() > CGROUP_NAME_MAX {
            return Err(EINVAL);
        }

        // Verify parent exists and is active
        let parent_idx = self.find_idx(parent_id).ok_or(ENOENT)?;
        let parent_controllers = self.cgroups[parent_idx].subtree_control;
        let mut parent_path_buf = [0u8; CGROUP_PATH_MAX];
        let p_len_orig = self.cgroups[parent_idx].path_len as usize;
        parent_path_buf[..p_len_orig].copy_from_slice(&self.cgroups[parent_idx].path[..p_len_orig]);
        let parent_is_root = p_len_orig == 1 && parent_path_buf[0] == b'/';

        // Check if name already exists under this parent
        for cg in self.cgroups.iter() {
            if cg.active && cg.parent_id == Some(parent_id) && cg.name_as_str() == name {
                return Err(EEXIST);
            }
        }

        // Find free slot
        let mut free_idx = None;
        for (i, cg) in self.cgroups.iter().enumerate() {
            if !cg.active {
                free_idx = Some(i);
                break;
            }
        }
        let slot = free_idx.ok_or(ENOMEM)?;

        let new_id = self.next_id;
        self.next_id += 1;

        let cg = &mut self.cgroups[slot];
        cg.id = new_id;
        cg.parent_id = Some(parent_id);
        cg.cgroup_type = CGROUP_TYPE_DOMAIN;
        cg.controllers_available = parent_controllers;
        cg.subtree_control = 0; // Child starts with subtree_control empty
        cg.frozen = false;
        cg.proc_count = 0;
        cg.memory = MemoryControllerState::empty();
        cg.cpu = CpuControllerState::empty();
        cg.io = IoControllerState::empty();
        cg.pids = PidsControllerState::empty();
        cg.active = true;

        // Copy name
        cg.name_len = name_bytes.len() as u8;
        cg.name[..name_bytes.len()].copy_from_slice(name_bytes);

        // Build path: e.g. parent is "/" -> "/name", else "parent/name"
        let mut path_buf = [0u8; CGROUP_PATH_MAX];
        let mut p_len = 0;
        if !parent_is_root {
            path_buf[..p_len_orig].copy_from_slice(&parent_path_buf[..p_len_orig]);
            p_len = p_len_orig;
        }
        if p_len < CGROUP_PATH_MAX {
            path_buf[p_len] = b'/';
            p_len += 1;
        }
        let copy_name = name_bytes.len().min(CGROUP_PATH_MAX - p_len);
        path_buf[p_len..p_len + copy_name].copy_from_slice(&name_bytes[..copy_name]);
        p_len += copy_name;

        cg.path_len = p_len as u8;
        cg.path[..p_len].copy_from_slice(&path_buf[..p_len]);

        Ok(new_id)
    }

    /// Destroy an empty child cgroup.
    pub fn destroy_cgroup(&mut self, cgroup_id: u32) -> Result<(), isize> {
        if cgroup_id == ROOT_CGROUP_ID {
            return Err(EBUSY); // Cannot delete root
        }

        let idx = self.find_idx(cgroup_id).ok_or(ENOENT)?;

        // Verify no active processes attached
        if self.cgroups[idx].proc_count > 0 {
            return Err(EBUSY);
        }

        // Verify no child cgroups exist
        for cg in self.cgroups.iter() {
            if cg.active && cg.parent_id == Some(cgroup_id) {
                return Err(EBUSY);
            }
        }

        self.cgroups[idx] = CgroupNode::empty();
        Ok(())
    }

    /// Attach a process (PID) to a target cgroup.
    pub fn attach_task(&mut self, target_cgroup_id: u32, pid: u32) -> Result<(), isize> {
        if pid as usize >= self.proc_map.len() {
            return Err(EINVAL);
        }

        let target_idx = self.find_idx(target_cgroup_id).ok_or(ENOENT)?;

        // Check PIDs limit on target cgroup
        self.cgroups[target_idx].pids.charge()?;

        // If previously attached, remove from old cgroup
        if let Some(old_cg_id) = self.proc_map[pid as usize] {
            if old_cg_id == target_cgroup_id {
                // Already here, undo charge
                self.cgroups[target_idx].pids.uncharge();
                return Ok(());
            }
            if let Some(old_idx) = self.find_idx(old_cg_id) {
                self.remove_proc_from_node(old_idx, pid);
                self.cgroups[old_idx].pids.uncharge();
            }
        }

        // Add to target node
        let node = &mut self.cgroups[target_idx];
        if node.proc_count >= MAX_PROCS_PER_CGROUP {
            node.pids.uncharge();
            return Err(ENOMEM);
        }
        node.procs[node.proc_count] = pid;
        node.proc_count += 1;
        self.proc_map[pid as usize] = Some(target_cgroup_id);

        Ok(())
    }

    /// Remove an exiting process from its cgroup.
    pub fn remove_task(&mut self, pid: u32) {
        if pid as usize >= self.proc_map.len() {
            return;
        }
        if let Some(cg_id) = self.proc_map[pid as usize] {
            if let Some(idx) = self.find_idx(cg_id) {
                self.remove_proc_from_node(idx, pid);
                self.cgroups[idx].pids.uncharge();
                let charged = self.proc_memory[pid as usize];
                if charged > 0 {
                    self.cgroups[idx].memory.uncharge(charged, true);
                    self.proc_memory[pid as usize] = 0;
                }
            }
            self.proc_map[pid as usize] = None;
        }
    }

    /// Hook for fork/clone: inherits parent's cgroup and checks pids.max.
    pub fn on_fork(&mut self, parent_pid: u32, child_pid: u32) -> Result<(), isize> {
        let parent_cg_id = if (parent_pid as usize) < self.proc_map.len() {
            self.proc_map[parent_pid as usize].unwrap_or(ROOT_CGROUP_ID)
        } else {
            ROOT_CGROUP_ID
        };

        self.attach_task(parent_cg_id, child_pid)
    }

    /// Set subtree_control string (e.g. "+cpu +memory +io +pids" or "-io").
    pub fn set_subtree_control(&mut self, cgroup_id: u32, s: &str) -> Result<(), isize> {
        let idx = self.find_idx(cgroup_id).ok_or(ENOENT)?;
        let available = self.cgroups[idx].controllers_available;
        let mut control = self.cgroups[idx].subtree_control;

        for tok in s.split_ascii_whitespace() {
            if tok.is_empty() {
                continue;
            }
            let is_add = tok.starts_with('+');
            let is_sub = tok.starts_with('-');
            if !is_add && !is_sub {
                return Err(EINVAL);
            }
            let name = &tok[1..];
            let flag = match name {
                "cpu" => CONTROLLER_CPU,
                "memory" => CONTROLLER_MEMORY,
                "io" => CONTROLLER_IO,
                "pids" => CONTROLLER_PIDS,
                _ => return Err(EINVAL),
            };

            if is_add {
                if (available & flag) == 0 {
                    return Err(EINVAL); // Not available in this cgroup
                }
                control |= flag;
            } else {
                control &= !flag;
            }
        }

        self.cgroups[idx].subtree_control = control;

        // Propagate availability to immediate children
        for i in 0..MAX_CGROUPS {
            if self.cgroups[i].active && self.cgroups[i].parent_id == Some(cgroup_id) {
                self.cgroups[i].controllers_available = control;
            }
        }

        Ok(())
    }

    /// Set cgroup.freeze (true = freeze, false = unfreeze).
    pub fn set_freeze(&mut self, cgroup_id: u32, freeze: bool) -> Result<(), isize> {
        let idx = self.find_idx(cgroup_id).ok_or(ENOENT)?;
        self.cgroups[idx].frozen = freeze;
        Ok(())
    }

    /// Memory Allocation Charge: checks memory.max and memory.high.
    /// If memory.max is exceeded, triggers OOM: finds heaviest process, kills it, increments oom_kill.
    pub fn charge_memory(&mut self, pid: u32, bytes: u64, is_anon: bool) -> Result<(), isize> {
        let cg_id = if (pid as usize) < self.proc_map.len() {
            self.proc_map[pid as usize].unwrap_or(ROOT_CGROUP_ID)
        } else {
            ROOT_CGROUP_ID
        };

        // Walk up from current cgroup to root, charging each
        let mut cur_id = Some(cg_id);
        let mut charged_nodes = [0u32; 8];
        let mut charged_count = 0;

        while let Some(id) = cur_id {
            if let Some(idx) = self.find_idx(id) {
                match self.cgroups[idx].memory.charge(bytes, is_anon) {
                    Ok(_throttled) => {
                        charged_nodes[charged_count] = id;
                        charged_count += 1;
                        cur_id = self.cgroups[idx].parent_id;
                    }
                    Err(ENOMEM) => {
                        // Rollback already charged nodes
                        for i in 0..charged_count {
                            if let Some(r_idx) = self.find_idx(charged_nodes[i]) {
                                self.cgroups[r_idx].memory.uncharge(bytes, is_anon);
                            }
                        }

                        // Trigger OOM: record OOM kill
                        self.trigger_oom(idx);
                        return Err(ENOMEM);
                    }
                    Err(e) => return Err(e),
                }
            } else {
                break;
            }
        }

        if (pid as usize) < self.proc_memory.len() {
            self.proc_memory[pid as usize] = self.proc_memory[pid as usize].saturating_add(bytes);
        }

        Ok(())
    }

    /// Memory Uncharge
    pub fn uncharge_memory(&mut self, pid: u32, bytes: u64, is_anon: bool) {
        let cg_id = if (pid as usize) < self.proc_map.len() {
            self.proc_map[pid as usize].unwrap_or(ROOT_CGROUP_ID)
        } else {
            ROOT_CGROUP_ID
        };

        let mut cur_id = Some(cg_id);
        while let Some(id) = cur_id {
            if let Some(idx) = self.find_idx(id) {
                self.cgroups[idx].memory.uncharge(bytes, is_anon);
                cur_id = self.cgroups[idx].parent_id;
            } else {
                break;
            }
        }

        if (pid as usize) < self.proc_memory.len() {
            self.proc_memory[pid as usize] = self.proc_memory[pid as usize].saturating_sub(bytes);
        }
    }

    /// OOM Killer policy handler
    fn trigger_oom(&mut self, cgroup_idx: usize) {
        let node = &mut self.cgroups[cgroup_idx];
        if node.memory.oom_group {
            node.memory.events_oom_group_kill += 1;
            node.memory.events_oom_kill += node.proc_count as u64;
            // Clear all processes from group
            for i in 0..node.proc_count {
                let p = node.procs[i];
                if (p as usize) < self.proc_map.len() {
                    self.proc_map[p as usize] = None;
                    self.proc_memory[p as usize] = 0;
                }
            }
            node.proc_count = 0;
            node.pids.current_pids = 0;
        } else {
            // Pick heaviest memory user
            let mut victim_pid = None;
            let mut max_usage = 0;
            for i in 0..node.proc_count {
                let p = node.procs[i];
                let usage = if (p as usize) < self.proc_memory.len() { self.proc_memory[p as usize] } else { 0 };
                if usage >= max_usage {
                    max_usage = usage;
                    victim_pid = Some(p);
                }
            }

            if let Some(v_pid) = victim_pid {
                node.memory.events_oom_kill += 1;
                self.remove_task(v_pid);
            }
        }
    }

    /// CPU Bandwidth Enforcement: called on scheduler quantum/tick.
    /// Returns true if process is permitted to run, or false if throttled.
    pub fn on_scheduler_tick(
        &mut self,
        pid: u32,
        current_tick: u64,
        delta_usec: u64,
        is_user: bool,
    ) -> bool {
        let cg_id = if (pid as usize) < self.proc_map.len() {
            self.proc_map[pid as usize].unwrap_or(ROOT_CGROUP_ID)
        } else {
            ROOT_CGROUP_ID
        };

        // If cgroup is frozen, process must not run
        if let Some(idx) = self.find_idx(cg_id) {
            if self.cgroups[idx].frozen {
                return false;
            }
        }

        // Walk up from current cgroup to root, checking cpu.max quota
        let mut cur_id = Some(cg_id);
        let mut allowed = true;

        while let Some(id) = cur_id {
            if let Some(idx) = self.find_idx(id) {
                let ok = self.cgroups[idx].cpu.on_tick(current_tick, delta_usec, is_user);
                if !ok {
                    allowed = false;
                }
                cur_id = self.cgroups[idx].parent_id;
            } else {
                break;
            }
        }

        allowed
    }

    /// Block I/O Charge: checks and accounts io.max.
    pub fn charge_io(
        &mut self,
        pid: u32,
        current_tick: u64,
        major: u32,
        minor: u32,
        is_write: bool,
        bytes: u64,
    ) -> Result<(), isize> {
        let cg_id = if (pid as usize) < self.proc_map.len() {
            self.proc_map[pid as usize].unwrap_or(ROOT_CGROUP_ID)
        } else {
            ROOT_CGROUP_ID
        };

        let mut cur_id = Some(cg_id);
        while let Some(id) = cur_id {
            if let Some(idx) = self.find_idx(id) {
                self.cgroups[idx].io.check_and_charge(current_tick, major, minor, is_write, bytes)?;
                cur_id = self.cgroups[idx].parent_id;
            } else {
                break;
            }
        }

        Ok(())
    }

    /// Lookup cgroup by normalized path (e.g. "/sys/fs/cgroup/prod/db" or "/prod/db").
    pub fn lookup_by_path(&self, path: &str) -> Option<u32> {
        let trimmed = path.trim();
        let rel = if let Some(stripped) = trimmed.strip_prefix("/sys/fs/cgroup") {
            if stripped.is_empty() { "/" } else { stripped }
        } else {
            trimmed
        };

        for cg in self.cgroups.iter() {
            if cg.active && cg.path_as_str() == rel {
                return Some(cg.id);
            }
        }
        None
    }

    // -------------------------------------------------------------------------
    // Formatters for cgroupfs pseudo-files
    // -------------------------------------------------------------------------

    pub fn format_controllers(&self, cgroup_id: u32, buf: &mut [u8]) -> usize {
        let idx = match self.find_idx(cgroup_id) {
            Some(i) => i,
            None => return 0,
        };
        format_controller_mask(self.cgroups[idx].controllers_available, buf)
    }

    pub fn format_subtree_control(&self, cgroup_id: u32, buf: &mut [u8]) -> usize {
        let idx = match self.find_idx(cgroup_id) {
            Some(i) => i,
            None => return 0,
        };
        format_controller_mask(self.cgroups[idx].subtree_control, buf)
    }

    pub fn format_procs(&self, cgroup_id: u32, buf: &mut [u8]) -> usize {
        let idx = match self.find_idx(cgroup_id) {
            Some(i) => i,
            None => return 0,
        };
        let node = &self.cgroups[idx];
        let mut offset = 0;
        for i in 0..node.proc_count {
            offset += format_u64_line(node.procs[i] as u64, &mut buf[offset..]);
        }
        offset
    }

    pub fn format_events(&self, cgroup_id: u32, buf: &mut [u8]) -> usize {
        let idx = match self.find_idx(cgroup_id) {
            Some(i) => i,
            None => return 0,
        };
        let node = &self.cgroups[idx];
        let mut offset = 0;
        let populated = if node.proc_count > 0 { 1 } else { 0 };
        offset += append_kv(b"populated", populated, &mut buf[offset..]);
        let frozen = if node.frozen { 1 } else { 0 };
        offset += append_kv(b"frozen", frozen, &mut buf[offset..]);
        offset
    }

    pub fn format_freeze(&self, cgroup_id: u32, buf: &mut [u8]) -> usize {
        let idx = match self.find_idx(cgroup_id) {
            Some(i) => i,
            None => return 0,
        };
        if self.cgroups[idx].frozen {
            copy_slice(b"1\n", buf)
        } else {
            copy_slice(b"0\n", buf)
        }
    }

    pub fn format_stat(&self, cgroup_id: u32, buf: &mut [u8]) -> usize {
        let _idx = match self.find_idx(cgroup_id) {
            Some(i) => i,
            None => return 0,
        };
        let mut descendants = 0;
        for cg in self.cgroups.iter() {
            if cg.active && cg.parent_id == Some(cgroup_id) {
                descendants += 1;
            }
        }
        let mut offset = 0;
        offset += append_kv(b"nr_descendants", descendants, &mut buf[offset..]);
        offset += append_kv(b"nr_dying_descendants", 0, &mut buf[offset..]);
        offset
    }

    pub fn format_type(&self, cgroup_id: u32, buf: &mut [u8]) -> usize {
        let idx = match self.find_idx(cgroup_id) {
            Some(i) => i,
            None => return 0,
        };
        if self.cgroups[idx].cgroup_type == CGROUP_TYPE_THREADED {
            copy_slice(b"threaded\n", buf)
        } else {
            copy_slice(b"domain\n", buf)
        }
    }

    // Internal helper
    pub fn find_idx(&self, cgroup_id: u32) -> Option<usize> {
        for (i, cg) in self.cgroups.iter().enumerate() {
            if cg.active && cg.id == cgroup_id {
                return Some(i);
            }
        }
        None
    }

    fn remove_proc_from_node(&mut self, idx: usize, pid: u32) {
        let node = &mut self.cgroups[idx];
        let mut found = None;
        for i in 0..node.proc_count {
            if node.procs[i] == pid {
                found = Some(i);
                break;
            }
        }
        if let Some(pos) = found {
            for i in pos..node.proc_count.saturating_sub(1) {
                node.procs[i] = node.procs[i + 1];
            }
            node.procs[node.proc_count - 1] = 0;
            node.proc_count -= 1;
        }
    }
}

fn format_controller_mask(mask: u32, buf: &mut [u8]) -> usize {
    let mut offset = 0;
    let mut first = true;
    if (mask & CONTROLLER_CPU) != 0 {
        offset += copy_slice(b"cpu", &mut buf[offset..]);
        first = false;
    }
    if (mask & CONTROLLER_MEMORY) != 0 {
        if !first && offset < buf.len() { buf[offset] = b' '; offset += 1; }
        offset += copy_slice(b"memory", &mut buf[offset..]);
        first = false;
    }
    if (mask & CONTROLLER_IO) != 0 {
        if !first && offset < buf.len() { buf[offset] = b' '; offset += 1; }
        offset += copy_slice(b"io", &mut buf[offset..]);
        first = false;
    }
    if (mask & CONTROLLER_PIDS) != 0 {
        if !first && offset < buf.len() { buf[offset] = b' '; offset += 1; }
        offset += copy_slice(b"pids", &mut buf[offset..]);
    }
    if offset < buf.len() {
        buf[offset] = b'\n';
        offset += 1;
    }
    offset
}
