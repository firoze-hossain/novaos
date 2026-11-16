//! userland/server/novactl/src/cgroup.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Cgroups v2 Container Controller
//!
//! Configures unified hierarchy control groups for containers:
//!   - Creates /sys/fs/cgroup/novactl/<container-id>
//!   - Applies memory.max, cpu.max, io.max, and pids.max
//!   - Attaches process via cgroup.procs
//!   - Collects live resource telemetry for `novactl stats`

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct ContainerMetrics {
    pub memory_current_bytes: u64,
    pub memory_max_bytes: u64,
    pub cpu_usage_usec: u64,
    pub cpu_periods: u64,
    pub cpu_throttled: u64,
    pub pids_current: u64,
    pub pids_max: u64,
    pub io_read_bytes: u64,
    pub io_write_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct CgroupController {
    pub cgroup_dir: PathBuf,
    pub simulated: bool,
}

impl CgroupController {
    pub fn new(container_id: &str) -> Self {
        if let Ok(custom) = std::env::var("NOVACTL_CGROUP_DIR") {
            return Self {
                cgroup_dir: PathBuf::from(custom).join(container_id),
                simulated: true,
            };
        }

        let base_cgroup = Path::new("/sys/fs/cgroup");
        let can_write = if base_cgroup.exists() {
            let test_target = base_cgroup.join("novactl");
            fs::create_dir_all(&test_target).is_ok()
        } else {
            false
        };

        let (cgroup_dir, simulated) = if can_write {
            (base_cgroup.join("novactl").join(container_id), false)
        } else {
            (std::env::temp_dir().join("novactl_cgroups").join(container_id), true)
        };

        Self {
            cgroup_dir,
            simulated,
        }
    }

    /// Set up cgroup directory and write resource limits.
    pub fn setup(
        &self,
        memory_max: Option<u64>,
        cpu_quota: Option<u64>,
        cpu_period: u64,
        pids_limit: Option<u64>,
        io_max: Option<&str>,
    ) -> Result<(), String> {
        fs::create_dir_all(&self.cgroup_dir)
            .map_err(|e| format!("Failed to create cgroup dir {:?}: {}", self.cgroup_dir, e))?;

        // 1. Memory limit
        if let Some(mem) = memory_max {
            self.write_file("memory.max", &format!("{}\n", mem))?;
        }

        // 2. CPU quota and period
        if let Some(quota) = cpu_quota {
            self.write_file("cpu.max", &format!("{} {}\n", quota, cpu_period))?;
        }

        // 3. PIDs limit
        if let Some(pids) = pids_limit {
            self.write_file("pids.max", &format!("{}\n", pids))?;
        }

        // 4. IO limit
        if let Some(io) = io_max {
            self.write_file("io.max", &format!("{}\n", io))?;
        }

        Ok(())
    }

    /// Attach a process ID to this container's cgroup.
    pub fn attach_pid(&self, pid: u32) -> Result<(), String> {
        self.write_file("cgroup.procs", &format!("{}\n", pid))
    }

    /// Freeze all processes in the container cgroup.
    pub fn freeze(&self) -> Result<(), String> {
        self.write_file("cgroup.freeze", "1\n")
    }

    /// Unfreeze processes in the container cgroup.
    pub fn unfreeze(&self) -> Result<(), String> {
        self.write_file("cgroup.freeze", "0\n")
    }

    /// Query live telemetry from cgroup pseudo-files.
    pub fn read_metrics(&self) -> ContainerMetrics {
        let mut m = ContainerMetrics::default();

        if let Ok(mem_str) = self.read_file("memory.current") {
            m.memory_current_bytes = mem_str.trim().parse::<u64>().unwrap_or(0);
        }
        if let Ok(mem_max_str) = self.read_file("memory.max") {
            m.memory_max_bytes = mem_max_str.trim().parse::<u64>().unwrap_or(u64::MAX);
        }
        if let Ok(pids_str) = self.read_file("pids.current") {
            m.pids_current = pids_str.trim().parse::<u64>().unwrap_or(0);
        }
        if let Ok(pids_max_str) = self.read_file("pids.max") {
            m.pids_max = pids_max_str.trim().parse::<u64>().unwrap_or(u64::MAX);
        }
        if let Ok(cpu_stat) = self.read_file("cpu.stat") {
            for line in cpu_stat.lines() {
                let mut parts = line.split_whitespace();
                if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                    match k {
                        "usage_usec" => m.cpu_usage_usec = v.parse().unwrap_or(0),
                        "nr_periods" => m.cpu_periods = v.parse().unwrap_or(0),
                        "nr_throttled" => m.cpu_throttled = v.parse().unwrap_or(0),
                        _ => {}
                    }
                }
            }
        }

        m
    }

    /// Clean up cgroup directory on container deletion.
    pub fn cleanup(&self) -> Result<(), String> {
        if self.cgroup_dir.exists() {
            fs::remove_dir_all(&self.cgroup_dir)
                .map_err(|e| format!("Failed to remove cgroup dir {:?}: {}", self.cgroup_dir, e))?;
        }
        Ok(())
    }

    fn write_file(&self, filename: &str, content: &str) -> Result<(), String> {
        let file_path = self.cgroup_dir.join(filename);
        fs::write(&file_path, content)
            .map_err(|e| format!("Failed to write to {:?}: {}", file_path, e))
    }

    fn read_file(&self, filename: &str) -> Result<String, String> {
        let file_path = self.cgroup_dir.join(filename);
        fs::read_to_string(&file_path)
            .map_err(|e| format!("Failed to read {:?}: {}", file_path, e))
    }
}
