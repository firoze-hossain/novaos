//! kernel/rust/procman.rs - Month 5: Activity & Performance Monitor Subsystem.
//!
//! Provides the kernel-side scheduler and process-table introspection engine for
//! Month 5 ("Activity/performance monitor app"). Gathers system-wide telemetry
//! (CPU utilization, SMP core count, uptime, physical memory metrics via PMM)
//! and per-process telemetry (PID, name, state, UID/GID, CPU ticks, CPU %, memory)
//! backing `SYS_PROC_INFO` (84).
//!
//! # Architecture & Guarantees
//!
//! 1. Read-Only Safety:
//!    Non-destructive observation of scheduler and process table state. No locks
//!    held across user boundaries; safe to query from unprivileged ring-3 apps.
//!
//! 2. Zero Dynamic Allocation:
//!    Populates fixed-size ABI structures (`nova_sys_activity_t`, `nova_proc_table_t`,
//!    `nova_proc_entry_t`) directly into caller-provided memory.
//!
//! 3. SMP & CPU Accounting:
//!    Integrates with `rlimit`'s windowed tick accounting (`rl.win_used`) and
//!    `apic`'s online CPU enumeration for real-time CPU percentages.

#![cfg_attr(not(test), no_std)]

pub const NOVA_SYS_PROC_INFO: u32 = 84;

pub const NOVA_PROC_OP_GET_SYSINFO: u32 = 1;
pub const NOVA_PROC_OP_GET_TABLE: u32 = 2;
pub const NOVA_PROC_OP_GET_ENTRY: u32 = 3;

pub const NOVA_PROC_MAX_ENTRIES: usize = 64;
pub const NOVA_PROC_NAME_MAX: usize = 32;

pub const NOVA_PROC_STATE_UNUSED: u32 = 0;
pub const NOVA_PROC_STATE_ALLOCATING: u32 = 1;
pub const NOVA_PROC_STATE_READY: u32 = 2;
pub const NOVA_PROC_STATE_RUNNING: u32 = 3;
pub const NOVA_PROC_STATE_TERMINATED: u32 = 4;

/// System-Wide Activity & Performance Metrics matching `nova_proc_abi.h`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NovaSysActivity {
    pub uptime_ticks: u32,
    pub cpu_count: u32,
    pub cpu_usage_pct: u32,
    pub total_frames: u32,
    pub used_frames: u32,
    pub free_frames: u32,
    pub total_processes: u32,
    pub running_processes: u32,
    pub user_processes: u32,
    pub kernel_processes: u32,
    pub context_switches: u32,
}

impl NovaSysActivity {
    pub const fn empty() -> Self {
        NovaSysActivity {
            uptime_ticks: 0,
            cpu_count: 1,
            cpu_usage_pct: 0,
            total_frames: 0,
            used_frames: 0,
            free_frames: 0,
            total_processes: 0,
            running_processes: 0,
            user_processes: 0,
            kernel_processes: 0,
            context_switches: 0,
        }
    }
}

/// Single Process Activity Record matching `nova_proc_abi.h`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NovaProcEntry {
    pub pid: u32,
    pub name: [u8; NOVA_PROC_NAME_MAX],
    pub state: u32,
    pub is_user: u32,
    pub uid: u32,
    pub gid: u32,
    pub cpu_ticks: u32,
    pub cpu_percent: u32,
    pub mem_pages: u32,
    pub mem_bytes: u32,
    pub throttled: u32,
    pub off_cpu: u32,
}

impl NovaProcEntry {
    pub const fn empty() -> Self {
        NovaProcEntry {
            pid: 0,
            name: [0; NOVA_PROC_NAME_MAX],
            state: NOVA_PROC_STATE_UNUSED,
            is_user: 0,
            uid: 0,
            gid: 0,
            cpu_ticks: 0,
            cpu_percent: 0,
            mem_pages: 0,
            mem_bytes: 0,
            throttled: 0,
            off_cpu: 0,
        }
    }
}

/// Process Table Snapshot matching `nova_proc_abi.h`.
#[repr(C)]
pub struct NovaProcTable {
    pub count: u32,
    pub entries: [NovaProcEntry; NOVA_PROC_MAX_ENTRIES],
}

impl NovaProcTable {
    pub const fn empty() -> Self {
        NovaProcTable {
            count: 0,
            entries: [NovaProcEntry::empty(); NOVA_PROC_MAX_ENTRIES],
        }
    }
}

#[repr(C)]
pub struct PmmStats {
    pub total_frames: u32,
    pub used_frames: u32,
    pub free_frames: u32,
}

#[cfg(not(test))]
extern "C" {
    fn timer_get_ticks() -> u32;
    fn pmm_get_stats(out: *mut PmmStats);
    fn rust_smp_online_cpu_count() -> u32;
    fn process_table_capacity() -> i32;
    fn process_entry_get_info(index: i32, out_entry: *mut NovaProcEntry) -> bool;
}

// ---------------------------------------------------------------------------
// Host Test Stubs
// ---------------------------------------------------------------------------
#[cfg(test)]
static mut MOCK_TICKS: u32 = 1250;
#[cfg(test)]
unsafe fn timer_get_ticks() -> u32 {
    MOCK_TICKS
}
#[cfg(test)]
unsafe fn pmm_get_stats(out: *mut PmmStats) {
    if !out.is_null() {
        (*out).total_frames = 131072; // 512MB
        (*out).used_frames = 16384;   // 64MB
        (*out).free_frames = 114688;  // 448MB
    }
}
#[cfg(test)]
unsafe fn rust_smp_online_cpu_count() -> u32 {
    2
}
#[cfg(test)]
unsafe fn process_table_capacity() -> i32 {
    4
}
#[cfg(test)]
unsafe fn process_entry_get_info(index: i32, out_entry: *mut NovaProcEntry) -> bool {
    if out_entry.is_null() {
        return false;
    }
    match index {
        0 => {
            let e = &mut *out_entry;
            e.pid = 0;
            e.name[..4].copy_from_slice(b"idle");
            e.name[4] = 0;
            e.state = NOVA_PROC_STATE_RUNNING;
            e.is_user = 0;
            e.cpu_ticks = 800;
            e.cpu_percent = 70; // 70% idle
            e.mem_pages = 4;
            e.mem_bytes = 16384;
            true
        }
        1 => {
            let e = &mut *out_entry;
            e.pid = 1;
            e.name[..8].copy_from_slice(b"novainit");
            e.name[8] = 0;
            e.state = NOVA_PROC_STATE_READY;
            e.is_user = 1;
            e.uid = 0;
            e.cpu_ticks = 150;
            e.cpu_percent = 15;
            e.mem_pages = 32;
            e.mem_bytes = 131072;
            true
        }
        2 => {
            let e = &mut *out_entry;
            e.pid = 2;
            e.name[..9].copy_from_slice(b"SHELL.ELF");
            e.name[9] = 0;
            e.state = NOVA_PROC_STATE_READY;
            e.is_user = 1;
            e.uid = 0;
            e.cpu_ticks = 90;
            e.cpu_percent = 10;
            e.mem_pages = 48;
            e.mem_bytes = 196608;
            true
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn rust_proc_get_sysinfo(out: *mut NovaSysActivity) -> i32 {
    if out.is_null() {
        return -14; // -EFAULT
    }

    let mut stats = PmmStats {
        total_frames: 0,
        used_frames: 0,
        free_frames: 0,
    };
    unsafe {
        pmm_get_stats(&mut stats);
    }

    let uptime = unsafe { timer_get_ticks() };
    let cpus = unsafe { rust_smp_online_cpu_count() }.max(1);
    let cap = unsafe { process_table_capacity() };

    let mut tot_proc = 0u32;
    let mut run_proc = 0u32;
    let mut usr_proc = 0u32;
    let mut kern_proc = 0u32;
    let mut active_cpu_sum = 0u32;
    let mut idle_cpu_pct = 0u32;

    let mut tmp_entry = NovaProcEntry::empty();

    for i in 0..cap {
        let ok = unsafe { process_entry_get_info(i, &mut tmp_entry) };
        if ok {
            tot_proc += 1;
            if tmp_entry.state == NOVA_PROC_STATE_RUNNING || tmp_entry.state == NOVA_PROC_STATE_READY {
                run_proc += 1;
            }
            if tmp_entry.is_user != 0 {
                usr_proc += 1;
            } else {
                kern_proc += 1;
            }

            // Check if this task is the idle thread
            let is_idle = tmp_entry.name[0] == b'i'
                && tmp_entry.name[1] == b'd'
                && tmp_entry.name[2] == b'l'
                && tmp_entry.name[3] == b'e'
                && (tmp_entry.name[4] == 0 || tmp_entry.name[4] == b' ');

            if is_idle {
                idle_cpu_pct = tmp_entry.cpu_percent.min(100);
            } else {
                active_cpu_sum += tmp_entry.cpu_percent;
            }
        }
    }

    // CPU utilization calculation:
    // If idle thread had valid accounting, total CPU load is 100 - idle%.
    // Otherwise fall back to sum of active processes normalized across cores.
    let cpu_load = if idle_cpu_pct > 0 {
        100 - idle_cpu_pct
    } else {
        (active_cpu_sum / cpus).min(100)
    };

    unsafe {
        (*out).uptime_ticks = uptime;
        (*out).cpu_count = cpus;
        (*out).cpu_usage_pct = cpu_load;
        (*out).total_frames = stats.total_frames;
        (*out).used_frames = stats.used_frames;
        (*out).free_frames = stats.free_frames;
        (*out).total_processes = tot_proc;
        (*out).running_processes = run_proc;
        (*out).user_processes = usr_proc;
        (*out).kernel_processes = kern_proc;
        (*out).context_switches = uptime * 2; // ~2 context switches per tick estimate
    }

    0
}

#[no_mangle]
pub extern "C" fn rust_proc_get_table(out: *mut NovaProcTable) -> i32 {
    if out.is_null() {
        return -14; // -EFAULT
    }

    let cap = unsafe { process_table_capacity() };
    let mut count = 0usize;

    for i in 0..cap {
        if count >= NOVA_PROC_MAX_ENTRIES {
            break;
        }

        unsafe {
            let slot_ptr = &mut (*out).entries[count];
            let ok = process_entry_get_info(i, slot_ptr);
            if ok {
                count += 1;
            }
        }
    }

    unsafe {
        (*out).count = count as u32;
    }

    0
}

#[no_mangle]
pub extern "C" fn rust_proc_get_entry(pid: u32, out: *mut NovaProcEntry) -> i32 {
    if out.is_null() {
        return -14; // -EFAULT
    }

    let cap = unsafe { process_table_capacity() };
    let mut tmp = NovaProcEntry::empty();

    for i in 0..cap {
        let ok = unsafe { process_entry_get_info(i, &mut tmp) };
        if ok && tmp.pid == pid {
            unsafe {
                *out = tmp;
            }
            return 0;
        }
    }

    -3 // -ESRCH (No such process)
}

/// In-kernel self-test for Month 5 Activity & Performance Monitor subsystem.
#[no_mangle]
pub extern "C" fn rust_procman_selftest() -> i32 {
    let mut sysinfo = NovaSysActivity::empty();
    if rust_proc_get_sysinfo(&mut sysinfo) != 0 {
        return 1;
    }

    if sysinfo.cpu_count == 0 || sysinfo.total_frames == 0 || sysinfo.total_processes == 0 {
        return 2;
    }

    let mut table = NovaProcTable::empty();
    if rust_proc_get_table(&mut table) != 0 {
        return 3;
    }

    if table.count == 0 {
        return 4;
    }

    // Check that at least one entry has non-zero name
    let first_has_name = table.entries[0].name[0] != 0;
    if !first_has_name {
        return 5;
    }

    // Query specific PID of first process
    let first_pid = table.entries[0].pid;
    let mut single_entry = NovaProcEntry::empty();
    if rust_proc_get_entry(first_pid, &mut single_entry) != 0 {
        return 6;
    }

    if single_entry.pid != first_pid {
        return 7;
    }

    // Hostile queries
    if rust_proc_get_sysinfo(core::ptr::null_mut()) != -14 {
        return 8;
    }
    if rust_proc_get_table(core::ptr::null_mut()) != -14 {
        return 9;
    }
    if rust_proc_get_entry(99999, &mut single_entry) != -3 {
        return 10;
    }

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proc_get_sysinfo() {
        let mut sysinfo = NovaSysActivity::empty();
        let rc = rust_proc_get_sysinfo(&mut sysinfo);
        assert_eq!(rc, 0);
        assert_eq!(sysinfo.uptime_ticks, 1250);
        assert_eq!(sysinfo.cpu_count, 2);
        assert_eq!(sysinfo.total_processes, 3);
        assert_eq!(sysinfo.user_processes, 2);
        assert_eq!(sysinfo.kernel_processes, 1);
        assert_eq!(sysinfo.total_frames, 131072);
        assert_eq!(sysinfo.cpu_usage_pct, 30); // 100 - 70% idle
    }

    #[test]
    fn test_proc_get_table() {
        let mut table = NovaProcTable::empty();
        let rc = rust_proc_get_table(&mut table);
        assert_eq!(rc, 0);
        assert_eq!(table.count, 3);
        assert_eq!(table.entries[0].pid, 0);
        assert_eq!(&table.entries[0].name[..4], b"idle");
        assert_eq!(table.entries[1].pid, 1);
        assert_eq!(&table.entries[1].name[..8], b"novainit");
        assert_eq!(table.entries[2].pid, 2);
        assert_eq!(&table.entries[2].name[..9], b"SHELL.ELF");
    }

    #[test]
    fn test_proc_get_entry() {
        let mut entry = NovaProcEntry::empty();
        assert_eq!(rust_proc_get_entry(1, &mut entry), 0);
        assert_eq!(entry.pid, 1);
        assert_eq!(&entry.name[..8], b"novainit");
        assert_eq!(entry.uid, 0);

        assert_eq!(rust_proc_get_entry(999, &mut entry), -3);
    }

    #[test]
    fn test_procman_selftest() {
        assert_eq!(rust_procman_selftest(), 0);
    }
}
