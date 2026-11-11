//! kernel/src/sched/thread.rs - Thread Control Block (TCB) & Priority Classes
//!
//! NovaOS 2.0 (Month 1 - Foundation): Preemptive Tickless SMP Scheduler with Priority Classes.
//!
//! Features:
//!   - 5 Priority Classes: Realtime, Interactive, Normal, Batch, Idle.
//!   - Dynamic priority boost (Windows NT style) for responsive UI and I/O wakeups.
//!   - Virtual runtime accounting (Linux CFS style) for fair Normal class execution.
//!   - Thread Control Block (TCB) with CPU affinity masks (2 to 32 cores).
//!   - Extended CPU register preservation: x86_64 XSAVE/XRSTOR 832-byte 64-byte-aligned state (SSE/AVX).

#![allow(dead_code)]

// -----------------------------------------------------------------------------
// Priority Classes & Levels (0 .. 31)
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PriorityClass {
    /// Background/Idle threads (priorities 0..3), executes only when core is idle.
    Idle = 0,
    /// Batch/Compute-bound threads (priorities 4..7), long quantums (e.g. 25ms), low preemption.
    Batch = 1,
    /// Standard application threads (priorities 8..15), CFS virtual runtime fair queuing.
    Normal = 2,
    /// User interface, shell, compositor, and audio threads (priorities 16..23), dynamic boost.
    Interactive = 3,
    /// Hard real-time kernel tasks and critical drivers (priorities 24..31), fixed highest priority.
    Realtime = 4,
}

impl Default for PriorityClass {
    fn default() -> Self {
        Self::Normal
    }
}

impl PriorityClass {
    /// Returns the default base priority for this class.
    pub const fn default_base_priority(&self) -> u8 {
        match self {
            Self::Idle => 0,
            Self::Batch => 4,
            Self::Normal => 10,
            Self::Interactive => 18,
            Self::Realtime => 28,
        }
    }

    /// Returns the maximum priority ceiling for dynamic boosting within this class.
    pub const fn priority_ceiling(&self) -> u8 {
        match self {
            Self::Idle => 3,
            Self::Batch => 7,
            Self::Normal => 15,
            Self::Interactive => 23,
            Self::Realtime => 31,
        }
    }

    /// Returns standard quantum in timer ticks (e.g. at 1000Hz: 1 tick = 1ms).
    pub const fn default_quantum_ticks(&self) -> u32 {
        match self {
            Self::Idle => 50,        // 50ms
            Self::Batch => 25,       // 25ms
            Self::Normal => 10,      // 10ms
            Self::Interactive => 5,  // 5ms (responsive low-latency)
            Self::Realtime => 2,     // 2ms (frequent preemption checks)
        }
    }

    /// Relative weight for CFS virtual runtime calculations ($W$).
    pub const fn weight(&self) -> u64 {
        match self {
            Self::Idle => 16,
            Self::Batch => 256,
            Self::Normal => 1024,
            Self::Interactive => 4096,
            Self::Realtime => 16384,
        }
    }
}

// -----------------------------------------------------------------------------
// Thread State & Sleep Reasons
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SleepReason {
    Timer { wake_deadline_ns: u64 },
    IoWait,
    IpcWait,
    LockWait,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ThreadState {
    Ready,
    Running { cpu_id: u8 },
    Blocked(SleepReason),
    Terminated,
}

impl Default for ThreadState {
    fn default() -> Self {
        Self::Ready
    }
}

// -----------------------------------------------------------------------------
// Thread Context & SSE/AVX XSAVE State
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, Default)]
#[repr(C)]
pub struct ThreadContext {
    pub rip: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rflags: u64,
    pub cr3: u64,
    pub fs_base: u64,
    pub gs_base: u64,
    pub rax: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
}

impl ThreadContext {
    pub const fn new() -> Self {
        Self {
            rip: 0, rsp: 0, rbp: 0, rbx: 0, r12: 0, r13: 0, r14: 0, r15: 0,
            rflags: 0, cr3: 0, fs_base: 0, gs_base: 0, rax: 0, rcx: 0, rdx: 0,
            rsi: 0, rdi: 0, r8: 0, r9: 0, r10: 0, r11: 0,
        }
    }
}

/// 832-byte 64-byte-aligned storage for x86_64 XSAVE / FXSAVE extended CPU state (SSE + AVX).
#[derive(Copy, Clone)]
#[repr(C, align(64))]
pub struct XSaveArea {
    pub data: [u8; 832],
}

impl Default for XSaveArea {
    fn default() -> Self {
        Self { data: [0; 832] }
    }
}

impl XSaveArea {
    pub const fn new() -> Self {
        Self { data: [0; 832] }
    }

    /// Initializes standard x87/SSE control word state (FCW=0x037F, MXCSR=0x1F80).
    pub fn init_standard_fpu(&mut self) {
        self.data[0] = 0x7F;
        self.data[1] = 0x03;
        self.data[24] = 0x80;
        self.data[25] = 0x1F;
    }
}

// -----------------------------------------------------------------------------
// Thread Control Block (TCB)
// -----------------------------------------------------------------------------

/// Complete Thread Control Block (TCB) managing execution parameters, SMP affinity,
/// CFS vruntime, and dynamic priority boosting.
#[derive(Clone, Copy)]
pub struct ThreadControlBlock {
    pub tid: u64,
    pub pid: u32,
    pub name: [u8; 16],
    pub name_len: usize,
    pub state: ThreadState,
    pub priority_class: PriorityClass,
    pub base_priority: u8,
    pub effective_priority: u8,
    pub quantum_remaining_ticks: u32,
    pub total_cpu_time_ns: u64,
    pub vruntime_ns: u64,
    pub affinity_mask: u64,
    pub assigned_cpu: u8,
    pub context: ThreadContext,
    pub xsave_area_addr: u64,
}

impl Default for ThreadControlBlock {
    fn default() -> Self {
        Self::new(0, 0, b"idle", PriorityClass::Idle, 0xFFFF_FFFF_FFFF_FFFF)
    }
}

impl ThreadControlBlock {
    pub const fn const_empty(cpu_id: u8) -> Self {
        Self {
            tid: 0,
            pid: 0,
            name: [0u8; 16],
            name_len: 0,
            state: ThreadState::Ready,
            priority_class: PriorityClass::Idle,
            base_priority: 0,
            effective_priority: 0,
            quantum_remaining_ticks: 1,
            total_cpu_time_ns: 0,
            vruntime_ns: 0,
            affinity_mask: 1u64 << (cpu_id & 63),
            assigned_cpu: cpu_id,
            context: ThreadContext::new(),
            xsave_area_addr: 0,
        }
    }

    pub fn new(
        tid: u64,
        pid: u32,
        name_bytes: &[u8],
        priority_class: PriorityClass,
        affinity_mask: u64,
    ) -> Self {
        let mut name = [0u8; 16];
        let name_len = core::cmp::min(name_bytes.len(), 15);
        name[..name_len].copy_from_slice(&name_bytes[..name_len]);

        let base_prio = priority_class.default_base_priority();
        let quantum = priority_class.default_quantum_ticks();

        Self {
            tid,
            pid,
            name,
            name_len,
            state: ThreadState::Ready,
            priority_class,
            base_priority: base_prio,
            effective_priority: base_prio,
            quantum_remaining_ticks: quantum,
            total_cpu_time_ns: 0,
            vruntime_ns: 0,
            affinity_mask: if affinity_mask == 0 { 0x0000_0000_FFFF_FFFF } else { affinity_mask },
            assigned_cpu: 0,
            context: ThreadContext::default(),
            xsave_area_addr: 0,
        }
    }

    /// Checks if this thread is eligible to execute on the given CPU core index (0..31).
    #[inline]
    pub fn can_run_on_cpu(&self, cpu_id: u8) -> bool {
        if cpu_id >= 64 {
            false
        } else {
            (self.affinity_mask & (1u64 << cpu_id)) != 0
        }
    }

    /// Applies a dynamic priority boost (e.g. upon user I/O event, IPC receive, or timer wake).
    /// Modeled after Windows NT dynamic boost: elevates effective priority up to the class ceiling.
    pub fn boost_priority(&mut self, boost_points: u8) {
        if self.priority_class == PriorityClass::Realtime || self.priority_class == PriorityClass::Idle {
            return;
        }
        let ceiling = self.priority_class.priority_ceiling();
        let new_prio = self.effective_priority.saturating_add(boost_points);
        self.effective_priority = core::cmp::min(new_prio, ceiling);
        self.quantum_remaining_ticks = self.priority_class.default_quantum_ticks();
    }

    /// Decays boosted priority by 1 point back towards base priority upon quantum expiration.
    pub fn decay_priority(&mut self) {
        if self.effective_priority > self.base_priority {
            self.effective_priority -= 1;
        }
        self.quantum_remaining_ticks = self.priority_class.default_quantum_ticks();
    }

    /// Consumes CPU quantum. Returns `true` if the quantum has expired and thread should yield/preempt.
    pub fn consume_quantum(&mut self, ticks: u32) -> bool {
        if self.quantum_remaining_ticks <= ticks {
            self.quantum_remaining_ticks = 0;
            true
        } else {
            self.quantum_remaining_ticks -= ticks;
            false
        }
    }

    /// Charges virtual runtime (CFS model):
    /// `vruntime += delta_ns * (1024 / weight)`.
    pub fn charge_vruntime(&mut self, delta_ns: u64) {
        self.total_cpu_time_ns = self.total_cpu_time_ns.saturating_add(delta_ns);
        let weight = self.priority_class.weight();
        let scaled_delta = (delta_ns * 1024) / weight;
        self.vruntime_ns = self.vruntime_ns.saturating_add(scaled_delta);
    }
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_priority_classes() {
        assert_eq!(PriorityClass::Realtime.default_base_priority(), 28);
        assert_eq!(PriorityClass::Interactive.default_base_priority(), 18);
        assert_eq!(PriorityClass::Normal.default_base_priority(), 10);
        assert_eq!(PriorityClass::Batch.default_base_priority(), 4);
        assert_eq!(PriorityClass::Idle.default_base_priority(), 0);

        assert!(PriorityClass::Interactive.default_quantum_ticks() < PriorityClass::Batch.default_quantum_ticks());
    }

    #[test]
    fn test_dynamic_priority_boost_and_decay() {
        let mut t = ThreadControlBlock::new(1, 100, b"ui_shell", PriorityClass::Interactive, 0x3);
        assert_eq!(t.base_priority, 18);
        assert_eq!(t.effective_priority, 18);

        t.boost_priority(4);
        assert_eq!(t.effective_priority, 22);

        t.boost_priority(10);
        assert_eq!(t.effective_priority, 23);

        t.decay_priority();
        assert_eq!(t.effective_priority, 22);
        t.decay_priority();
        assert_eq!(t.effective_priority, 21);
    }

    #[test]
    fn test_cfs_vruntime_charging() {
        let mut t_norm = ThreadControlBlock::new(2, 101, b"worker", PriorityClass::Normal, 0x1);
        let mut t_interactive = ThreadControlBlock::new(3, 102, b"gui", PriorityClass::Interactive, 0x1);

        t_norm.charge_vruntime(1_000_000);
        t_interactive.charge_vruntime(1_000_000);

        assert!(t_interactive.vruntime_ns < t_norm.vruntime_ns);
    }

    #[test]
    fn test_affinity_and_xsave() {
        let mut t = ThreadControlBlock::new(4, 103, b"pinned_task", PriorityClass::Realtime, 0b1010);
        assert!(!t.can_run_on_cpu(0));
        assert!(t.can_run_on_cpu(1));
        assert!(!t.can_run_on_cpu(2));
        assert!(t.can_run_on_cpu(3));

        let mut xsave = XSaveArea::new();
        xsave.init_standard_fpu();
        assert_eq!(xsave.data[0], 0x7F);
        assert_eq!(xsave.data[1], 0x03);
        t.xsave_area_addr = &xsave as *const _ as u64;
        assert_ne!(t.xsave_area_addr, 0);
    }
}
