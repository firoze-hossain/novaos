//! kernel/src/fs/cgroupfs.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): CgroupFS Virtual Filesystem
//!
//! Exposes the Control Groups v2 unified hierarchy as a synthetic filesystem
//! mounted at `/sys/fs/cgroup`:
//!   - Reading control files generates standard ASCII metrics and configurations.
//!   - Writing control files modifies limits (memory.max, cpu.max, io.max, pids.max)
//!     or migrates processes (cgroup.procs).
//!   - mkdir creates a new child cgroup node.
//!   - rmdir removes an empty child cgroup node.

#![allow(dead_code)]

use crate::security::cgroups::v2::{
    CgroupManager, CONTROLLER_CPU, CONTROLLER_IO, CONTROLLER_MEMORY, CONTROLLER_PIDS,
    ROOT_CGROUP_ID,
};
use crate::security::cgroups::GLOBAL_CGROUPS;

pub const CGROUPFS_MOUNT_POINT: &str = "/sys/fs/cgroup";

pub const ENOENT: isize = -2;
pub const EINVAL: isize = -22;
pub const EBUSY: isize  = -16;
pub const ENOMEM: isize = -12;
pub const EPERM: isize  = -1;

/// Parsed cgroupfs target: (cgroup_id, control_file_name)
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CgroupFsTarget<'a> {
    Directory(u32),
    File(u32, &'a str),
}

pub struct CgroupFs;

impl CgroupFs {
    /// Read content of a cgroupfs file or directory into `buf`.
    /// Returns the number of bytes read, or a negative error code.
    pub fn read_file(path: &str, buf: &mut [u8]) -> Result<usize, isize> {
        let mut mgr = GLOBAL_CGROUPS.lock();
        let target = resolve_target(&mgr, path)?;

        match target {
            CgroupFsTarget::Directory(cg_id) => {
                // List directory entries
                Self::list_directory(&mgr, cg_id, buf)
            }
            CgroupFsTarget::File(cg_id, filename) => {
                Self::read_control_file(&mgr, cg_id, filename, buf)
            }
        }
    }

    /// Write data to a cgroupfs control file.
    /// Returns number of bytes written, or a negative error code.
    pub fn write_file(path: &str, data: &[u8]) -> Result<usize, isize> {
        let data_str = core::str::from_utf8(data).map_err(|_| EINVAL)?;
        let mut mgr = GLOBAL_CGROUPS.lock();
        let target = resolve_target(&mgr, path)?;

        match target {
            CgroupFsTarget::Directory(_) => Err(EINVAL),
            CgroupFsTarget::File(cg_id, filename) => {
                Self::write_control_file(&mut mgr, cg_id, filename, data_str)?;
                Ok(data.len())
            }
        }
    }

    /// Create a child cgroup directory (e.g. `/sys/fs/cgroup/prod`).
    pub fn mkdir(path: &str) -> Result<(), isize> {
        let (parent_path, child_name) = split_path(path)?;
        let mut mgr = GLOBAL_CGROUPS.lock();
        let parent_id = mgr.lookup_by_path(parent_path).ok_or(ENOENT)?;
        mgr.create_cgroup(parent_id, child_name).map(|_| ())
    }

    /// Remove an empty child cgroup directory.
    pub fn rmdir(path: &str) -> Result<(), isize> {
        let mut mgr = GLOBAL_CGROUPS.lock();
        let cg_id = mgr.lookup_by_path(path).ok_or(ENOENT)?;
        mgr.destroy_cgroup(cg_id)
    }

    fn list_directory(mgr: &CgroupManager, cg_id: u32, buf: &mut [u8]) -> Result<usize, isize> {
        let idx = mgr.find_idx(cg_id).ok_or(ENOENT)?;
        let mut offset = 0;

        // Core cgroup control files
        offset += append_line(b"cgroup.controllers", &mut buf[offset..]);
        offset += append_line(b"cgroup.subtree_control", &mut buf[offset..]);
        offset += append_line(b"cgroup.procs", &mut buf[offset..]);
        offset += append_line(b"cgroup.events", &mut buf[offset..]);
        offset += append_line(b"cgroup.freeze", &mut buf[offset..]);
        offset += append_line(b"cgroup.type", &mut buf[offset..]);
        offset += append_line(b"cgroup.stat", &mut buf[offset..]);

        let avail = mgr.cgroups[idx].controllers_available;
        if (avail & CONTROLLER_MEMORY) != 0 {
            offset += append_line(b"memory.max", &mut buf[offset..]);
            offset += append_line(b"memory.high", &mut buf[offset..]);
            offset += append_line(b"memory.current", &mut buf[offset..]);
            offset += append_line(b"memory.stat", &mut buf[offset..]);
            offset += append_line(b"memory.oom.group", &mut buf[offset..]);
            offset += append_line(b"memory.events", &mut buf[offset..]);
        }
        if (avail & CONTROLLER_CPU) != 0 {
            offset += append_line(b"cpu.max", &mut buf[offset..]);
            offset += append_line(b"cpu.weight", &mut buf[offset..]);
            offset += append_line(b"cpu.stat", &mut buf[offset..]);
        }
        if (avail & CONTROLLER_IO) != 0 {
            offset += append_line(b"io.max", &mut buf[offset..]);
            offset += append_line(b"io.weight", &mut buf[offset..]);
            offset += append_line(b"io.stat", &mut buf[offset..]);
        }
        if (avail & CONTROLLER_PIDS) != 0 {
            offset += append_line(b"pids.max", &mut buf[offset..]);
            offset += append_line(b"pids.current", &mut buf[offset..]);
            offset += append_line(b"pids.events", &mut buf[offset..]);
        }

        // Child directories
        for cg in mgr.cgroups.iter() {
            if cg.active && cg.parent_id == Some(cg_id) {
                offset += append_line(cg.name_as_str().as_bytes(), &mut buf[offset..]);
            }
        }

        Ok(offset)
    }

    fn read_control_file(
        mgr: &CgroupManager,
        cg_id: u32,
        filename: &str,
        buf: &mut [u8],
    ) -> Result<usize, isize> {
        let idx = mgr.find_idx(cg_id).ok_or(ENOENT)?;
        let cg = &mgr.cgroups[idx];

        let len = match filename {
            "cgroup.controllers" => mgr.format_controllers(cg_id, buf),
            "cgroup.subtree_control" => mgr.format_subtree_control(cg_id, buf),
            "cgroup.procs" => mgr.format_procs(cg_id, buf),
            "cgroup.events" => mgr.format_events(cg_id, buf),
            "cgroup.freeze" => mgr.format_freeze(cg_id, buf),
            "cgroup.type" => mgr.format_type(cg_id, buf),
            "cgroup.stat" => mgr.format_stat(cg_id, buf),

            // Memory controller
            "memory.max" => cg.memory.format_max(buf),
            "memory.high" => cg.memory.format_high(buf),
            "memory.current" => cg.memory.format_current(buf),
            "memory.stat" => cg.memory.format_stat(buf),
            "memory.oom.group" => cg.memory.format_oom_group(buf),
            "memory.events" => cg.memory.format_events(buf),

            // CPU controller
            "cpu.max" => cg.cpu.format_max(buf),
            "cpu.weight" => cg.cpu.format_weight(buf),
            "cpu.stat" => cg.cpu.format_stat(buf),

            // IO controller
            "io.max" => cg.io.format_max(buf),
            "io.weight" => cg.io.format_weight(buf),
            "io.stat" => cg.io.format_stat(buf),

            // PIDs controller
            "pids.max" => cg.pids.format_max(buf),
            "pids.current" => cg.pids.format_current(buf),
            "pids.events" => cg.pids.format_events(buf),

            _ => return Err(ENOENT),
        };

        Ok(len)
    }

    fn write_control_file(
        mgr: &mut CgroupManager,
        cg_id: u32,
        filename: &str,
        data: &str,
    ) -> Result<(), isize> {
        let idx = mgr.find_idx(cg_id).ok_or(ENOENT)?;

        match filename {
            "cgroup.subtree_control" => mgr.set_subtree_control(cg_id, data),
            "cgroup.procs" => {
                let trimmed = data.trim();
                let pid = parse_u32(trimmed).ok_or(EINVAL)?;
                mgr.attach_task(cg_id, pid)
            }
            "cgroup.freeze" => {
                let trimmed = data.trim();
                if trimmed == "1" {
                    mgr.set_freeze(cg_id, true)
                } else if trimmed == "0" {
                    mgr.set_freeze(cg_id, false)
                } else {
                    Err(EINVAL)
                }
            }

            // Memory controller
            "memory.max" => mgr.cgroups[idx].memory.set_max(data),
            "memory.high" => mgr.cgroups[idx].memory.set_high(data),
            "memory.oom.group" => mgr.cgroups[idx].memory.set_oom_group(data),

            // CPU controller
            "cpu.max" => mgr.cgroups[idx].cpu.set_max(data),
            "cpu.weight" => mgr.cgroups[idx].cpu.set_weight(data),

            // IO controller
            "io.max" => mgr.cgroups[idx].io.set_max(data),
            "io.weight" => mgr.cgroups[idx].io.set_weight(data),

            // PIDs controller
            "pids.max" => mgr.cgroups[idx].pids.set_max(data),

            _ => Err(EPERM),
        }
    }
}

// -----------------------------------------------------------------------------
// Path Resolution Helpers
// -----------------------------------------------------------------------------

fn resolve_target<'a>(mgr: &CgroupManager, full_path: &'a str) -> Result<CgroupFsTarget<'a>, isize> {
    let trimmed = full_path.trim();
    let rel = if let Some(stripped) = trimmed.strip_prefix(CGROUPFS_MOUNT_POINT) {
        if stripped.is_empty() { "/" } else { stripped }
    } else {
        trimmed
    };

    // Check if path is directly a cgroup directory
    if let Some(cg_id) = mgr.lookup_by_path(rel) {
        return Ok(CgroupFsTarget::Directory(cg_id));
    }

    // Split into parent directory and filename
    let (dir_part, filename) = split_path(rel)?;
    let cg_id = mgr.lookup_by_path(dir_part).ok_or(ENOENT)?;
    Ok(CgroupFsTarget::File(cg_id, filename))
}

fn split_path(path: &str) -> Result<(&str, &str), isize> {
    let trimmed = path.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok(("/", ""));
    }
    match trimmed.rfind('/') {
        Some(idx) => {
            let dir = if idx == 0 { "/" } else { &trimmed[..idx] };
            let name = &trimmed[idx + 1..];
            Ok((dir, name))
        }
        None => Ok(("/", trimmed)),
    }
}

fn append_line(src: &[u8], dst: &mut [u8]) -> usize {
    let copy = src.len().min(dst.len());
    dst[..copy].copy_from_slice(&src[..copy]);
    let mut len = copy;
    if len < dst.len() {
        dst[len] = b'\n';
        len += 1;
    }
    len
}

fn parse_u32(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut val: u32 = 0;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(val)
}
