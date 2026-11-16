//! kernel/src/security/cgroups/mod.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Control Groups v2 Subsystem
//!
//! Unified hierarchy resource governance in pure Rust:
//!   - memory.max / memory.high watermarks, OOM killing, and oom_group.
//!   - cpu.max CFS bandwidth quotas, periods, and throttling.
//!   - io.max per-device rate limiting (rbps, wbps, riops, wiops).
//!   - pids.max process limits and fork-bomb defense.
//!   - Virtual filesystem mounted at /sys/fs/cgroup.

#![allow(dead_code, non_camel_case_types)]

pub mod memory;
pub mod cpu;
pub mod io;
pub mod pids;
pub mod v2;

pub use memory::{MemoryControllerState, CGROUP_UNLIMITED};
pub use cpu::CpuControllerState;
pub use io::IoControllerState;
pub use pids::PidsControllerState;
pub use v2::{
    CgroupManager, CgroupNode, ROOT_CGROUP_ID, MAX_CGROUPS, MAX_PROCS_PER_CGROUP,
    CONTROLLER_ALL, CONTROLLER_CPU, CONTROLLER_IO, CONTROLLER_MEMORY, CONTROLLER_PIDS,
    CGROUP_TYPE_DOMAIN, CGROUP_TYPE_THREADED,
};

use core::sync::atomic::{AtomicBool, Ordering};

// -----------------------------------------------------------------------------
// Synchronization SpinLock
// -----------------------------------------------------------------------------

#[cfg(not(test))]
use crate::spinlock::SpinLock;

#[cfg(test)]
pub struct SpinLock<T>(std::sync::Mutex<T>);

#[cfg(test)]
impl<T> SpinLock<T> {
    pub const fn new(val: T) -> Self {
        Self(std::sync::Mutex::new(val))
    }
    pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
        self.0.lock().unwrap()
    }
}

// -----------------------------------------------------------------------------
// Global Static Cgroup Instances
// -----------------------------------------------------------------------------

pub static GLOBAL_CGROUPS: SpinLock<CgroupManager> = SpinLock::new(CgroupManager::empty());
static CGROUPS_INITIALIZED: AtomicBool = AtomicBool::new(false);
static TEST_CGROUPS: SpinLock<CgroupManager> = SpinLock::new(CgroupManager::empty());

fn ensure_global_init(mgr: &mut CgroupManager) {
    if !CGROUPS_INITIALIZED.load(Ordering::Acquire) {
        mgr.init();
        CGROUPS_INITIALIZED.store(true, Ordering::Release);
    }
}

// -----------------------------------------------------------------------------
// Public Kernel Accessors
// -----------------------------------------------------------------------------

pub fn global_cgroup_create(parent_id: u32, name: &str) -> Result<u32, isize> {
    let mut mgr = GLOBAL_CGROUPS.lock();
    ensure_global_init(&mut mgr);
    mgr.create_cgroup(parent_id, name)
}

pub fn global_cgroup_destroy(cgroup_id: u32) -> Result<(), isize> {
    let mut mgr = GLOBAL_CGROUPS.lock();
    ensure_global_init(&mut mgr);
    mgr.destroy_cgroup(cgroup_id)
}

pub fn global_cgroup_attach(cgroup_id: u32, pid: u32) -> Result<(), isize> {
    let mut mgr = GLOBAL_CGROUPS.lock();
    ensure_global_init(&mut mgr);
    mgr.attach_task(cgroup_id, pid)
}

pub fn global_cgroup_on_fork(parent_pid: u32, child_pid: u32) -> isize {
    let mut mgr = GLOBAL_CGROUPS.lock();
    ensure_global_init(&mut mgr);
    match mgr.on_fork(parent_pid, child_pid) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

pub fn global_cgroup_on_exit(pid: u32) {
    let mut mgr = GLOBAL_CGROUPS.lock();
    ensure_global_init(&mut mgr);
    mgr.remove_task(pid);
}

pub fn global_cgroup_charge_memory(pid: u32, bytes: u64, is_anon: bool) -> isize {
    let mut mgr = GLOBAL_CGROUPS.lock();
    ensure_global_init(&mut mgr);
    match mgr.charge_memory(pid, bytes, is_anon) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

pub fn global_cgroup_uncharge_memory(pid: u32, bytes: u64, is_anon: bool) {
    let mut mgr = GLOBAL_CGROUPS.lock();
    ensure_global_init(&mut mgr);
    mgr.uncharge_memory(pid, bytes, is_anon);
}

pub fn global_cgroup_on_scheduler_tick(
    pid: u32,
    current_tick: u64,
    delta_usec: u64,
    is_user: bool,
) -> bool {
    let mut mgr = GLOBAL_CGROUPS.lock();
    ensure_global_init(&mut mgr);
    mgr.on_scheduler_tick(pid, current_tick, delta_usec, is_user)
}

pub fn global_cgroup_charge_io(
    pid: u32,
    current_tick: u64,
    major: u32,
    minor: u32,
    is_write: bool,
    bytes: u64,
) -> isize {
    let mut mgr = GLOBAL_CGROUPS.lock();
    ensure_global_init(&mut mgr);
    match mgr.charge_io(pid, current_tick, major, minor, is_write, bytes) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

// -----------------------------------------------------------------------------
// In-Kernel Self-Test Suite
// -----------------------------------------------------------------------------

pub fn run_cgroups_v2_selftest() -> bool {
    let mut guard = TEST_CGROUPS.lock();
    guard.reset();
    let mgr = &mut *guard;

    // 1. Root Cgroup Verification
    let root_idx = match mgr.find_idx(ROOT_CGROUP_ID) {
        Some(i) => i,
        None => return false,
    };
    if mgr.cgroups[root_idx].path_as_str() != "/"
        || (mgr.cgroups[root_idx].controllers_available & CONTROLLER_ALL) != CONTROLLER_ALL
        || mgr.cgroups[root_idx].proc_count != 1
    {
        return false;
    }

    // 2. Enable Subtree Controllers
    if mgr.set_subtree_control(ROOT_CGROUP_ID, "+cpu +memory +io +pids").is_err() {
        return false;
    }

    // 3. Create Child Cgroup: "container-alpha" (/container-alpha)
    let alpha_id = match mgr.create_cgroup(ROOT_CGROUP_ID, "container-alpha") {
        Ok(id) => id,
        Err(_) => return false,
    };
    let alpha_idx = match mgr.find_idx(alpha_id) {
        Some(i) => i,
        None => return false,
    };
    if mgr.cgroups[alpha_idx].path_as_str() != "/container-alpha" {
        return false;
    }
    // Child must inherit controllers from parent subtree_control
    if (mgr.cgroups[alpha_idx].controllers_available & CONTROLLER_ALL) != CONTROLLER_ALL {
        return false;
    }

    // 4. PIDs Controller Enforcement (pids.max = 2)
    if mgr.cgroups[alpha_idx].pids.set_max("2").is_err() {
        return false;
    }
    // Attach PID 101 -> ok
    if mgr.attach_task(alpha_id, 101).is_err() {
        return false;
    }
    // Attach PID 102 -> ok (current = 2)
    if mgr.attach_task(alpha_id, 102).is_err() {
        return false;
    }
    // Fork PID 103 from 101 -> MUST FAIL with EAGAIN (-11)
    if mgr.on_fork(101, 103) != Err(pids::EAGAIN) {
        return false;
    }
    if mgr.cgroups[alpha_idx].pids.events_max != 1 {
        return false;
    }

    // 5. CPU Bandwidth Controller (cpu.max = 50000 100000 = 50ms / 100ms period)
    if mgr.cgroups[alpha_idx].cpu.set_max("50000 100000").is_err() {
        return false;
    }
    // Run for 30ms (30,000 usec) at tick 10 -> permitted
    if !mgr.on_scheduler_tick(101, 10, 30_000, true) {
        return false;
    }
    // Run for another 25ms (25,000 usec) at tick 12 -> total 55ms > 50ms quota -> throttled!
    if mgr.on_scheduler_tick(101, 12, 25_000, true) {
        return false; // Must be throttled
    }
    // Advance tick to next period (100ms = 10 ticks later, e.g. tick 25) -> quota renewed, permitted!
    if !mgr.on_scheduler_tick(101, 25, 10_000, true) {
        return false;
    }

    // 6. Memory Controller Enforcement & OOM Killing
    // Set memory.max = 10MB (10485760 bytes), memory.high = 8MB (8388608 bytes)
    if mgr.cgroups[alpha_idx].memory.set_max("10485760").is_err() {
        return false;
    }
    if mgr.cgroups[alpha_idx].memory.set_high("8388608").is_err() {
        return false;
    }
    // Charge 6MB to PID 101 -> ok
    if mgr.charge_memory(101, 6 * 1024 * 1024, true).is_err() {
        return false;
    }
    // Charge 3MB to PID 102 -> total 9MB > 8MB (high watermark) -> ok with throttle warning
    if mgr.charge_memory(102, 3 * 1024 * 1024, true).is_err() {
        return false;
    }
    if mgr.cgroups[alpha_idx].memory.events_high < 1 {
        return false;
    }
    // Charge 2MB more -> total 11MB > 10MB (max watermark) -> MUST TRIGGER OOM KILLER and return -ENOMEM
    if mgr.charge_memory(102, 2 * 1024 * 1024, true) != Err(-12) {
        return false;
    }
    // OOM killer selected heaviest process (PID 101 with 6MB) and reaped it
    if mgr.cgroups[alpha_idx].memory.events_oom_kill < 1 {
        return false;
    }

    // 7. I/O Throttling Enforcement (io.max = 8:0 rbps=10485760 wbps=10485760)
    if mgr.cgroups[alpha_idx].io.set_max("8:0 rbps=10485760 wbps=10485760 riops=1000 wiops=1000").is_err() {
        return false;
    }
    // Write 5MB at tick 30 -> allowed
    if mgr.charge_io(102, 30, 8, 0, true, 5 * 1024 * 1024).is_err() {
        return false;
    }
    // Write another 6MB in same window -> total 11MB > 10MB -> throttled with -EBUSY
    if mgr.charge_io(102, 35, 8, 0, true, 6 * 1024 * 1024) != Err(-16) {
        return false;
    }

    // 8. Cgroup Freezing
    if mgr.set_freeze(alpha_id, true).is_err() {
        return false;
    }
    if mgr.on_scheduler_tick(102, 50, 5_000, true) {
        return false; // Frozen cgroup must not execute
    }
    if mgr.set_freeze(alpha_id, false).is_err() {
        return false;
    }

    // 9. Teardown and Cleanup
    // Exit PID 102
    mgr.remove_task(102);
    if mgr.cgroups[alpha_idx].proc_count != 0 {
        return false;
    }
    // Destroy empty cgroup
    if mgr.destroy_cgroup(alpha_id).is_err() {
        return false;
    }
    if mgr.cgroups[alpha_idx].active {
        return false;
    }

    true
}

// -----------------------------------------------------------------------------
// Host Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cgroups_v2_hierarchy_and_limits() {
        assert!(run_cgroups_v2_selftest());
    }

    #[test]
    fn test_memory_controller_format_and_reclaim() {
        let mut mem = MemoryControllerState::empty();
        assert_eq!(mem.set_max("52428800"), Ok(()));
        assert_eq!(mem.set_high("41943040"), Ok(()));
        assert_eq!(mem.charge(10485760, true), Ok(false));
        assert_eq!(mem.current_bytes, 10485760);

        let mut buf = [0u8; 128];
        let len = mem.format_current(&mut buf);
        assert_eq!(&buf[..len], b"10485760\n");

        let len_max = mem.format_max(&mut buf);
        assert_eq!(&buf[..len_max], b"52428800\n");
    }

    #[test]
    fn test_cpu_controller_cfs_quota() {
        let mut cpu = CpuControllerState::empty();
        assert_eq!(cpu.set_max("20000 100000"), Ok(()));
        assert_eq!(cpu.quota_usec, 20000);
        assert_eq!(cpu.period_usec, 100000);

        // Tick 0: 15ms
        assert!(cpu.on_tick(0, 15000, true));
        // Tick 2: 10ms -> 25ms total > 20ms quota -> throttled
        assert!(!cpu.on_tick(2, 10000, true));
        assert!(cpu.is_throttled);

        // Tick 11: 110ms later -> new period -> unthrottled
        assert!(cpu.on_tick(11, 5000, true));
        assert!(!cpu.is_throttled);
    }

    #[test]
    fn test_io_controller_rate_limit() {
        let mut io = IoControllerState::empty();
        assert_eq!(io.set_max("8:0 rbps=1000000 wbps=500000 riops=100 wiops=50"), Ok(()));
        assert_eq!(io.check_and_charge(0, 8, 0, true, 300000), Ok(()));
        assert_eq!(io.check_and_charge(10, 8, 0, true, 300000), Err(-16)); // 600K > 500K
    }

    #[test]
    fn test_pids_controller_limits() {
        let mut p = PidsControllerState::empty();
        assert_eq!(p.set_max("3"), Ok(()));
        assert_eq!(p.charge(), Ok(()));
        assert_eq!(p.charge(), Ok(()));
        assert_eq!(p.charge(), Ok(()));
        assert_eq!(p.charge(), Err(pids::EAGAIN)); // 4th fails
        assert_eq!(p.events_max, 1);
        p.uncharge();
        assert_eq!(p.charge(), Ok(())); // 3rd succeeds again
    }
}
