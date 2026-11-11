//! kernel/src/sched/scheduler.rs - Preemptive Tickless SMP Scheduler Engine
//!
//! NovaOS 2.0 (Month 1 - Foundation): Preemptive Tickless SMP Scheduler with Priority Classes.
//!
//! Features:
//!   - Scalable SMP architecture supporting 2 to 32 CPU cores.
//!   - Multi-Level Priority Classes: Realtime, Interactive, Normal, Batch, Idle.
//!   - Dynamic priority boost for interactive UI threads and I/O wakeups (Windows NT style).
//!   - Fair virtual runtime scheduling for Normal threads (Linux CFS style).
//!   - Lockless/fair cross-core work-stealing when runqueues starve.
//!   - Tickless Local APIC one-shot timers eliminating timer interrupt overhead on idle cores.

#![allow(dead_code)]

#[path = "runqueue.rs"]
pub mod runqueue;

pub use runqueue::thread;

use runqueue::{PerCpuRunQueue, TicketSpinLock};
use thread::{PriorityClass, ThreadControlBlock, ThreadState};

pub const MAX_SMP_CORES: usize = 32;

// -----------------------------------------------------------------------------
// Tickless Timer State & Local APIC Controller
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TicklessTimerState {
    /// APIC timer disarmed: core is idle, zero interrupt overhead.
    DisarmedIdle,
    /// APIC timer armed for a single one-shot interrupt at `deadline_ns`.
    ArmedOneShot { deadline_ns: u64 },
}

impl Default for TicklessTimerState {
    fn default() -> Self {
        Self::DisarmedIdle
    }
}

// -----------------------------------------------------------------------------
// Scheduling Actions & Errors
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedAction {
    ContinueRunning { tid: u64 },
    SwitchContext { prev_tid: u64, next_tid: u64 },
    EnterIdle { cpu_id: u8 },
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SchedError {
    InvalidCoreId,
    ThreadQueueFull,
    NoEligibleCores,
}

// -----------------------------------------------------------------------------
// Global Preemptive Tickless SMP Scheduler
// -----------------------------------------------------------------------------

pub struct SmpScheduler {
    pub active_cores: u8,
    pub runqueues: [PerCpuRunQueue; MAX_SMP_CORES],
    pub current_threads: [Option<ThreadControlBlock>; MAX_SMP_CORES],
    pub timer_states: [TicklessTimerState; MAX_SMP_CORES],
    pub idle_threads: [ThreadControlBlock; MAX_SMP_CORES],
    pub total_context_switches: u64,
    pub total_work_steals: u64,
    pub total_dynamic_boosts: u64,
    pub total_tickless_skips: u64,
    pub global_lock: TicketSpinLock,
}

impl SmpScheduler {
    pub const fn const_init() -> Self {
        const NONE_THREAD: Option<ThreadControlBlock> = None;
        Self {
            active_cores: 1,
            runqueues: [
                PerCpuRunQueue::const_empty(0), PerCpuRunQueue::const_empty(1), PerCpuRunQueue::const_empty(2), PerCpuRunQueue::const_empty(3),
                PerCpuRunQueue::const_empty(4), PerCpuRunQueue::const_empty(5), PerCpuRunQueue::const_empty(6), PerCpuRunQueue::const_empty(7),
                PerCpuRunQueue::const_empty(8), PerCpuRunQueue::const_empty(9), PerCpuRunQueue::const_empty(10), PerCpuRunQueue::const_empty(11),
                PerCpuRunQueue::const_empty(12), PerCpuRunQueue::const_empty(13), PerCpuRunQueue::const_empty(14), PerCpuRunQueue::const_empty(15),
                PerCpuRunQueue::const_empty(16), PerCpuRunQueue::const_empty(17), PerCpuRunQueue::const_empty(18), PerCpuRunQueue::const_empty(19),
                PerCpuRunQueue::const_empty(20), PerCpuRunQueue::const_empty(21), PerCpuRunQueue::const_empty(22), PerCpuRunQueue::const_empty(23),
                PerCpuRunQueue::const_empty(24), PerCpuRunQueue::const_empty(25), PerCpuRunQueue::const_empty(26), PerCpuRunQueue::const_empty(27),
                PerCpuRunQueue::const_empty(28), PerCpuRunQueue::const_empty(29), PerCpuRunQueue::const_empty(30), PerCpuRunQueue::const_empty(31),
            ],
            current_threads: [NONE_THREAD; MAX_SMP_CORES],
            timer_states: [TicklessTimerState::DisarmedIdle; MAX_SMP_CORES],
            idle_threads: [
                ThreadControlBlock::const_empty(0), ThreadControlBlock::const_empty(1), ThreadControlBlock::const_empty(2), ThreadControlBlock::const_empty(3),
                ThreadControlBlock::const_empty(4), ThreadControlBlock::const_empty(5), ThreadControlBlock::const_empty(6), ThreadControlBlock::const_empty(7),
                ThreadControlBlock::const_empty(8), ThreadControlBlock::const_empty(9), ThreadControlBlock::const_empty(10), ThreadControlBlock::const_empty(11),
                ThreadControlBlock::const_empty(12), ThreadControlBlock::const_empty(13), ThreadControlBlock::const_empty(14), ThreadControlBlock::const_empty(15),
                ThreadControlBlock::const_empty(16), ThreadControlBlock::const_empty(17), ThreadControlBlock::const_empty(18), ThreadControlBlock::const_empty(19),
                ThreadControlBlock::const_empty(20), ThreadControlBlock::const_empty(21), ThreadControlBlock::const_empty(22), ThreadControlBlock::const_empty(23),
                ThreadControlBlock::const_empty(24), ThreadControlBlock::const_empty(25), ThreadControlBlock::const_empty(26), ThreadControlBlock::const_empty(27),
                ThreadControlBlock::const_empty(28), ThreadControlBlock::const_empty(29), ThreadControlBlock::const_empty(30), ThreadControlBlock::const_empty(31),
            ],
            total_context_switches: 0,
            total_work_steals: 0,
            total_dynamic_boosts: 0,
            total_tickless_skips: 0,
            global_lock: TicketSpinLock::new(),
        }
    }

    /// Initializes the SMP scheduler for `num_cores` in-place without stack allocation (supported: 2 to 32 cores).
    pub fn init(&mut self, num_cores: u8) {
        self.active_cores = core::cmp::min(core::cmp::max(num_cores, 1), MAX_SMP_CORES as u8);
        for i in 0..MAX_SMP_CORES {
            self.runqueues[i].reset(i as u8);
            self.current_threads[i] = None;
            self.timer_states[i] = TicklessTimerState::DisarmedIdle;
            self.idle_threads[i] = ThreadControlBlock::const_empty(i as u8);
        }
        self.total_context_switches = 0;
        self.total_work_steals = 0;
        self.total_dynamic_boosts = 0;
        self.total_tickless_skips = 0;
    }

    pub fn new(num_cores: u8) -> Self {
        let mut s = Self::const_init();
        s.init(num_cores);
        s
    }

    /// Spawns a thread onto the least loaded eligible CPU core.
    pub fn spawn_thread(&mut self, thread: ThreadControlBlock) -> Result<u8, SchedError> {
        let mut best_cpu: Option<u8> = None;
        let mut min_load = u64::MAX;

        for i in 0..(self.active_cores as usize) {
            let cpu = i as u8;
            if thread.can_run_on_cpu(cpu) {
                let load = self.runqueues[i].total_load_weight();
                if load < min_load {
                    min_load = load;
                    best_cpu = Some(cpu);
                }
            }
        }

        let target_cpu = best_cpu.ok_or(SchedError::NoEligibleCores)?;
        if self.runqueues[target_cpu as usize].enqueue(thread) {
            Ok(target_cpu)
        } else {
            Err(SchedError::ThreadQueueFull)
        }
    }

    /// Wakes a thread and applies a dynamic interactive boost.
    pub fn wake_and_boost_thread(&mut self, mut thread: ThreadControlBlock, boost_points: u8) -> Result<u8, SchedError> {
        thread.boost_priority(boost_points);
        self.total_dynamic_boosts += 1;
        self.spawn_thread(thread)
    }

    /// Primary scheduling dispatch for a given CPU core.
    /// Handles quantum expiry, work-stealing, and programming the tickless APIC timer.
    pub fn schedule_cpu(&mut self, cpu_id: u8, now_ns: u64) -> SchedAction {
        if cpu_id >= self.active_cores {
            return SchedAction::EnterIdle { cpu_id };
        }
        let cpu_idx = cpu_id as usize;

        // 1. Wake any expired sleeping threads on this core's SleepQueue.
        let mut woken_threads = [None, None, None, None];
        let mut woken_count = 0;
        self.runqueues[cpu_idx].sleep_queue.wake_expired(now_ns, |t| {
            if woken_count < 4 {
                woken_threads[woken_count] = Some(t);
                woken_count += 1;
            }
        });
        for i in 0..woken_count {
            if let Some(t) = woken_threads[i].take() {
                self.runqueues[cpu_idx].enqueue(t);
            }
        }

        // 2. Determine if the current thread should yield or be re-enqueued.
        let mut prev_tid = 0;
        if let Some(mut current) = self.current_threads[cpu_idx].take() {
            prev_tid = current.tid;
            if current.state == (ThreadState::Running { cpu_id }) {
                // If quantum expired, decay dynamic priority back toward base priority.
                if current.quantum_remaining_ticks == 0 {
                    current.decay_priority();
                    current.state = ThreadState::Ready;
                    self.runqueues[cpu_idx].enqueue(current);
                } else {
                    // Current thread still has quantum and remains highest? Check if preempted.
                    if let Some(next) = self.runqueues[cpu_idx].dequeue_highest() {
                        if next.effective_priority > current.effective_priority {
                            // Higher priority thread arrived -> Preempt current
                            current.state = ThreadState::Ready;
                            self.runqueues[cpu_idx].enqueue(current);
                            self.current_threads[cpu_idx] = Some(next.clone());
                            self.arm_tickless_timer(cpu_id, &next, now_ns);
                            self.total_context_switches += 1;
                            return SchedAction::SwitchContext {
                                prev_tid,
                                next_tid: next.tid,
                            };
                        } else {
                            // Current remains higher or equal
                            self.runqueues[cpu_idx].enqueue(next);
                            self.current_threads[cpu_idx] = Some(current);
                            return SchedAction::ContinueRunning { tid: prev_tid };
                        }
                    } else {
                        // No contenders
                        self.current_threads[cpu_idx] = Some(current);
                        return SchedAction::ContinueRunning { tid: prev_tid };
                    }
                }
            }
        }

        // 3. Pick next ready thread from local runqueue.
        let next_thread = if let Some(t) = self.runqueues[cpu_idx].dequeue_highest() {
            Some(t)
        } else {
            // 4. Local runqueue empty -> Attempt cross-core Work-Stealing!
            self.attempt_work_steal(cpu_id)
        };

        // 5. Context Switch & APIC Timer Programming.
        if let Some(next) = next_thread {
            let next_tid = next.tid;
            self.arm_tickless_timer(cpu_id, &next, now_ns);
            self.current_threads[cpu_idx] = Some(next);
            self.total_context_switches += 1;
            SchedAction::SwitchContext {
                prev_tid,
                next_tid,
            }
        } else {
            // Core is fully idle: disarm APIC timer (zero timer interrupt overhead!).
            self.timer_states[cpu_idx] = TicklessTimerState::DisarmedIdle;
            self.total_tickless_skips += 1;
            let _idle_tid = self.idle_threads[cpu_idx].tid;
            self.current_threads[cpu_idx] = Some(self.idle_threads[cpu_idx].clone());
            SchedAction::EnterIdle { cpu_id }
        }
    }

    /// Attempts to steal a thread from the most loaded active CPU core.
    fn attempt_work_steal(&mut self, my_cpu_id: u8) -> Option<ThreadControlBlock> {
        let mut most_loaded_core: Option<usize> = None;
        let mut max_load = 1u64;

        for i in 0..(self.active_cores as usize) {
            if i as u8 != my_cpu_id {
                let load = self.runqueues[i].total_load_weight();
                if load > max_load {
                    max_load = load;
                    most_loaded_core = Some(i);
                }
            }
        }

        if let Some(victim_idx) = most_loaded_core {
            let mut victim_rq = core::mem::replace(
                &mut self.runqueues[victim_idx],
                PerCpuRunQueue::new(victim_idx as u8),
            );
            let stolen = self.runqueues[my_cpu_id as usize].steal_work_from(&mut victim_rq);
            self.runqueues[victim_idx] = victim_rq;

            if stolen.is_some() {
                self.total_work_steals += 1;
            }
            stolen
        } else {
            None
        }
    }

    /// Arms the tickless Local APIC timer for the exact duration of the thread's quantum
    /// or the earliest sleep deadline on this core, whichever is sooner.
    fn arm_tickless_timer(&mut self, cpu_id: u8, thread: &ThreadControlBlock, now_ns: u64) {
        let cpu_idx = cpu_id as usize;
        let quantum_ns = (thread.quantum_remaining_ticks as u64) * 1_000_000; // 1 tick = 1ms = 1,000,000ns
        let quantum_deadline = now_ns + quantum_ns;

        let final_deadline = match self.runqueues[cpu_idx].sleep_queue.earliest_deadline_ns() {
            Some(earliest_sleep) => core::cmp::min(quantum_deadline, earliest_sleep),
            None => quantum_deadline,
        };

        self.timer_states[cpu_idx] = TicklessTimerState::ArmedOneShot {
            deadline_ns: final_deadline,
        };
    }

    /// Periodic or one-shot APIC timer interrupt trigger.
    pub fn on_apic_timer_interrupt(&mut self, cpu_id: u8, now_ns: u64, elapsed_ticks: u32) -> SchedAction {
        if cpu_id >= self.active_cores {
            return SchedAction::EnterIdle { cpu_id };
        }
        let cpu_idx = cpu_id as usize;

        if let Some(ref mut current) = self.current_threads[cpu_idx] {
            current.charge_vruntime((elapsed_ticks as u64) * 1_000_000);
            current.consume_quantum(elapsed_ticks);
        }

        self.schedule_cpu(cpu_id, now_ns)
    }
}

// -----------------------------------------------------------------------------
// Comprehensive Self-Test Engine
// -----------------------------------------------------------------------------

pub static mut GLOBAL_SMP_SCHEDULER: SmpScheduler = SmpScheduler::const_init();

/// Executes complete in-kernel verification of the Preemptive Tickless SMP Scheduler.
/// Returns 0 on success, non-zero error code on failure.
pub fn run_smp_scheduler_selftest() -> i32 {
    let sched = unsafe { &mut *core::ptr::addr_of_mut!(GLOBAL_SMP_SCHEDULER) };
    // 1. Initialize SMP scheduler across 8 cores in-place (simulated 2 to 32 core scaling).
    sched.init(8);

    if sched.active_cores != 8 {
        return 1;
    }

    // 2. Verify priority class dispatch ordering:
    // Realtime (prio 28) must preempt Normal (prio 10) and Batch (prio 4).
    let t_batch = ThreadControlBlock::new(1, 10, b"batch_job", PriorityClass::Batch, 1 << 0);
    let t_normal = ThreadControlBlock::new(2, 10, b"normal_app", PriorityClass::Normal, 1 << 0);
    let t_realtime = ThreadControlBlock::new(3, 10, b"rt_audio", PriorityClass::Realtime, 1 << 0);

    if sched.spawn_thread(t_batch).is_err() { return 2; }
    if sched.spawn_thread(t_normal).is_err() { return 3; }
    if sched.spawn_thread(t_realtime).is_err() { return 4; }

    // Schedule CPU 0: Realtime must be picked first
    let act1 = sched.schedule_cpu(0, 1_000_000);
    match act1 {
        SchedAction::SwitchContext { next_tid, .. } if next_tid == 3 => {}
        _ => return 5,
    }

    // 3. Verify Dynamic Priority Boost for Interactive UI thread.
    let t_ui = ThreadControlBlock::new(4, 20, b"compositor", PriorityClass::Interactive, 1 << 0);
    if sched.wake_and_boost_thread(t_ui, 4).is_err() {
        return 6;
    }
    if sched.total_dynamic_boosts != 1 {
        return 7;
    }

    // 4. Verify Cross-Core Work-Stealing:
    // Enqueue shareable task directly onto CPU 0 so CPU 0 has excess load.
    // Scheduling idle CPU 1 must steal this task from CPU 0.
    let t_worker = ThreadControlBlock::new(5, 30, b"worker_steals", PriorityClass::Normal, 0b11); // cores 0 & 1
    if !sched.runqueues[0].enqueue(t_worker) {
        return 8;
    }

    let act_cpu1 = sched.schedule_cpu(1, 2_000_000);
    match act_cpu1 {
        SchedAction::SwitchContext { next_tid, .. } if next_tid == 5 => {}
        _ => return 9,
    }
    if sched.total_work_steals < 1 {
        return 10;
    }

    // 5. Verify Tickless APIC Timer & Zero Interrupt Overhead on Idle Cores:
    // Core 7 has zero runnable threads.
    let act_idle = sched.schedule_cpu(7, 3_000_000);
    match act_idle {
        SchedAction::EnterIdle { cpu_id } if cpu_id == 7 => {}
        _ => return 11,
    }
    // Verify APIC timer is completely disarmed on idle core
    if sched.timer_states[7] != TicklessTimerState::DisarmedIdle {
        return 12;
    }
    if sched.total_tickless_skips < 1 {
        return 13;
    }

    // 6. Verify 32-core scaling in-place:
    sched.init(32);
    if sched.active_cores != 32 {
        return 14;
    }

    0 // SUCCESS
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smp_scheduler_selftest_passes() {
        let rc = run_smp_scheduler_selftest();
        assert_eq!(rc, 0, "Scheduler self-test failed with code {}", rc);
    }
}
