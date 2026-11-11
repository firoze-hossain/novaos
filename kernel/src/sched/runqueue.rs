//! kernel/src/sched/runqueue.rs - Per-CPU Multi-Level RunQueues, Ticket Spinlocks & Work-Stealing
//!
//! NovaOS 2.0 (Month 1 - Foundation): Preemptive Tickless SMP Scheduler with Priority Classes.
//!
//! Features:
//!   - Per-CPU Multi-Level Priority RunQueue with $O(1)$ priority bitmap scheduling.
//!   - Fair FIFO TicketSpinLock preventing thread starvation under multi-core contention.
//!   - Adaptive SleepQueue tracking exact tickless timer wake deadlines.
//!   - Lockless/fair Work-Stealing engine balancing load across 2 to 32 cores while honoring affinity.

#![allow(dead_code)]

#[path = "thread.rs"]
pub mod thread;

use core::sync::atomic::{AtomicU32, Ordering};
use thread::{PriorityClass, ThreadControlBlock, ThreadState};

pub const MAX_RUNQUEUE_LEN: usize = 8;
pub const MAX_SLEEP_LEN: usize = 4;
pub const PRIORITY_LEVELS: usize = 32;

// -----------------------------------------------------------------------------
// Ticket Spinlock (Fair FIFO SMP Mutual Exclusion)
// -----------------------------------------------------------------------------

/// Fair FIFO ticket spinlock. Guarantees deterministic, starvation-free locking
/// across multi-core systems (2 to 32 cores).
pub struct TicketSpinLock {
    now_serving: AtomicU32,
    next_ticket: AtomicU32,
}

impl Default for TicketSpinLock {
    fn default() -> Self {
        Self::new()
    }
}

impl TicketSpinLock {
    pub const fn new() -> Self {
        Self {
            now_serving: AtomicU32::new(0),
            next_ticket: AtomicU32::new(0),
        }
    }

    /// Acquires the lock, spinning until our ticket is called.
    pub fn lock(&self) -> u32 {
        let ticket = self.next_ticket.fetch_add(1, Ordering::SeqCst);
        while self.now_serving.load(Ordering::Acquire) != ticket {
            core::hint::spin_loop();
        }
        ticket
    }

    /// Releases the lock, serving the next ticket in line.
    pub fn unlock(&self) {
        self.now_serving.fetch_add(1, Ordering::Release);
    }

    /// Attempts to acquire the lock immediately without spinning.
    pub fn try_lock(&self) -> bool {
        let current = self.now_serving.load(Ordering::Acquire);
        self.next_ticket
            .compare_exchange(current, current.wrapping_add(1), Ordering::SeqCst, Ordering::Relaxed)
            .is_ok()
    }

    /// Checks if the lock is currently held.
    #[inline]
    pub fn is_locked(&self) -> bool {
        self.next_ticket.load(Ordering::Relaxed) != self.now_serving.load(Ordering::Relaxed)
    }
}

// -----------------------------------------------------------------------------
// Adaptive Sleep Queue (Tickless Deadline Tracker)
// -----------------------------------------------------------------------------

#[derive(Clone)]
pub struct SleepEntry {
    pub thread: ThreadControlBlock,
    pub wake_deadline_ns: u64,
}

/// Adaptive sleep queue managing blocked/sleeping threads and providing
/// exact earliest deadlines for programming tickless APIC one-shot timers.
pub struct SleepQueue {
    entries: [Option<SleepEntry>; MAX_SLEEP_LEN],
    count: usize,
}

impl Default for SleepQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl SleepQueue {
    pub const fn new() -> Self {
        const NONE_ENTRY: Option<SleepEntry> = None;
        Self {
            entries: [NONE_ENTRY; MAX_SLEEP_LEN],
            count: 0,
        }
    }

    pub fn reset(&mut self) {
        for slot in self.entries.iter_mut() {
            *slot = None;
        }
        self.count = 0;
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Puts a thread to sleep until `wake_deadline_ns`.
    pub fn sleep_until(&mut self, thread: ThreadControlBlock, wake_deadline_ns: u64) -> bool {
        for slot in self.entries.iter_mut() {
            if slot.is_none() {
                *slot = Some(SleepEntry {
                    thread,
                    wake_deadline_ns,
                });
                self.count += 1;
                return true;
            }
        }
        false
    }

    /// Returns the earliest wake deadline across all sleeping threads (for tickless APIC timer).
    pub fn earliest_deadline_ns(&self) -> Option<u64> {
        let mut min_deadline: Option<u64> = None;
        for slot in self.entries.iter() {
            if let Some(entry) = slot {
                min_deadline = Some(match min_deadline {
                    Some(m) => core::cmp::min(m, entry.wake_deadline_ns),
                    None => entry.wake_deadline_ns,
                });
            }
        }
        min_deadline
    }

    /// Wakes all threads whose deadlines have arrived (`wake_deadline_ns <= current_time_ns`).
    pub fn wake_expired<F: FnMut(ThreadControlBlock)>(
        &mut self,
        current_time_ns: u64,
        mut wake_fn: F,
    ) -> usize {
        let mut woken = 0;
        for slot in self.entries.iter_mut() {
            if let Some(entry) = slot {
                if entry.wake_deadline_ns <= current_time_ns {
                    let mut t = entry.thread.clone();
                    t.state = ThreadState::Ready;
                    // Interactive boost on timer wakeup for high responsiveness
                    t.boost_priority(2);
                    wake_fn(t);
                    *slot = None;
                    self.count -= 1;
                    woken += 1;
                }
            }
        }
        woken
    }
}

// -----------------------------------------------------------------------------
// Per-CPU Multi-Level RunQueue
// -----------------------------------------------------------------------------

/// Multi-Level Priority RunQueue dedicated to a specific CPU core.
/// Selects the highest priority runnable thread in $O(1)$ time via priority bitmap.
pub struct PerCpuRunQueue {
    pub cpu_id: u8,
    threads: [Option<ThreadControlBlock>; MAX_RUNQUEUE_LEN],
    prio_bitmap: u32,
    count: usize,
    pub sleep_queue: SleepQueue,
    pub lock: TicketSpinLock,
    pub total_switches: u64,
}

impl PerCpuRunQueue {
    pub const fn const_empty(cpu_id: u8) -> Self {
        const NONE_THREAD: Option<ThreadControlBlock> = None;
        Self {
            cpu_id,
            threads: [NONE_THREAD; MAX_RUNQUEUE_LEN],
            prio_bitmap: 0,
            count: 0,
            sleep_queue: SleepQueue::new(),
            lock: TicketSpinLock::new(),
            total_switches: 0,
        }
    }

    pub fn new(cpu_id: u8) -> Self {
        Self::const_empty(cpu_id)
    }

    pub fn reset(&mut self, cpu_id: u8) {
        self.cpu_id = cpu_id;
        for slot in self.threads.iter_mut() {
            *slot = None;
        }
        self.prio_bitmap = 0;
        self.count = 0;
        self.sleep_queue.reset();
        self.total_switches = 0;
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Enqueues a ready thread into this CPU's runqueue.
    pub fn enqueue(&mut self, mut thread: ThreadControlBlock) -> bool {
        thread.state = ThreadState::Ready;
        thread.assigned_cpu = self.cpu_id;
        let prio = thread.effective_priority as usize;

        for slot in self.threads.iter_mut() {
            if slot.is_none() {
                *slot = Some(thread);
                self.prio_bitmap |= 1u32 << prio;
                self.count += 1;
                return true;
            }
        }
        false
    }

    /// Dequeues the highest priority runnable thread ($O(1)$ via bitmap).
    pub fn dequeue_highest(&mut self) -> Option<ThreadControlBlock> {
        if self.prio_bitmap == 0 || self.count == 0 {
            return None;
        }

        // Find highest bit set (priorities 0..31, 31 is highest)
        let highest_prio = (31 - self.prio_bitmap.leading_zeros()) as u8;

        // Find matching thread with highest priority (or lowest vruntime for tie-break)
        let mut best_idx = None;
        let mut min_vruntime = u64::MAX;

        for (idx, slot) in self.threads.iter().enumerate() {
            if let Some(t) = slot {
                if t.effective_priority == highest_prio {
                    if t.vruntime_ns < min_vruntime {
                        min_vruntime = t.vruntime_ns;
                        best_idx = Some(idx);
                    }
                }
            }
        }

        if let Some(idx) = best_idx {
            let mut thread = self.threads[idx].take().unwrap();
            thread.state = ThreadState::Running { cpu_id: self.cpu_id };
            self.count -= 1;
            self.total_switches += 1;

            // Recompute bitmap bit for this priority level
            let mut still_has_prio = false;
            for slot in self.threads.iter() {
                if let Some(t) = slot {
                    if t.effective_priority == highest_prio {
                        still_has_prio = true;
                        break;
                    }
                }
            }
            if !still_has_prio {
                self.prio_bitmap &= !(1u32 << (highest_prio as usize));
            }

            Some(thread)
        } else {
            None
        }
    }

    /// Attempts to steal a thread from a victim runqueue to balance load across cores.
    /// Steals the lowest-priority runnable thread that honors this CPU's affinity mask.
    pub fn steal_work_from(&mut self, victim: &mut PerCpuRunQueue) -> Option<ThreadControlBlock> {
        if victim.count <= 1 {
            // Keep at least one thread on victim to avoid thrashing
            return None;
        }

        let my_cpu = self.cpu_id;
        let mut candidate_idx = None;
        let mut lowest_prio = u8::MAX;

        for (idx, slot) in victim.threads.iter().enumerate() {
            if let Some(t) = slot {
                if t.can_run_on_cpu(my_cpu) && t.priority_class != PriorityClass::Realtime {
                    if t.effective_priority < lowest_prio {
                        lowest_prio = t.effective_priority;
                        candidate_idx = Some(idx);
                    }
                }
            }
        }

        if let Some(idx) = candidate_idx {
            let mut stolen = victim.threads[idx].take().unwrap();
            victim.count -= 1;

            // Recompute victim priority bit
            let p = stolen.effective_priority as usize;
            let mut still_has = false;
            for slot in victim.threads.iter() {
                if let Some(t) = slot {
                    if t.effective_priority as usize == p {
                        still_has = true;
                        break;
                    }
                }
            }
            if !still_has {
                victim.prio_bitmap &= !(1u32 << p);
            }

            stolen.assigned_cpu = my_cpu;
            stolen.state = ThreadState::Ready;
            Some(stolen)
        } else {
            None
        }
    }

    /// Computes the total aggregate load weight of all runnable threads on this core.
    pub fn total_load_weight(&self) -> u64 {
        let mut weight = 0u64;
        for slot in self.threads.iter() {
            if let Some(t) = slot {
                weight = weight.saturating_add(t.priority_class.weight());
            }
        }
        weight
    }
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ticket_spinlock() {
        let lock = TicketSpinLock::new();
        assert!(!lock.is_locked());

        let t1 = lock.lock();
        assert!(lock.is_locked());
        assert_eq!(t1, 0);

        lock.unlock();
        assert!(!lock.is_locked());

        assert!(lock.try_lock());
        assert!(lock.is_locked());
        lock.unlock();
    }

    #[test]
    fn test_runqueue_priority_order() {
        let mut rq = PerCpuRunQueue::new(0);

        let t_low = ThreadControlBlock::new(1, 10, b"batch", PriorityClass::Batch, 0x1);
        let t_mid = ThreadControlBlock::new(2, 10, b"normal", PriorityClass::Normal, 0x1);
        let t_high = ThreadControlBlock::new(3, 10, b"realtime", PriorityClass::Realtime, 0x1);

        rq.enqueue(t_low);
        rq.enqueue(t_mid);
        rq.enqueue(t_high);

        assert_eq!(rq.len(), 3);

        // Highest priority (Realtime) must be dequeued first
        let first = rq.dequeue_highest().unwrap();
        assert_eq!(first.priority_class, PriorityClass::Realtime);

        // Normal second
        let second = rq.dequeue_highest().unwrap();
        assert_eq!(second.priority_class, PriorityClass::Normal);

        // Batch last
        let third = rq.dequeue_highest().unwrap();
        assert_eq!(third.priority_class, PriorityClass::Batch);

        assert!(rq.is_empty());
    }

    #[test]
    fn test_work_stealing_with_affinity() {
        let mut rq0 = PerCpuRunQueue::new(0);
        let mut rq1 = PerCpuRunQueue::new(1);

        // CPU 0 has 2 threads: one allowed on CPU 1, one pinned strictly to CPU 0
        let t_shareable = ThreadControlBlock::new(10, 20, b"shared", PriorityClass::Normal, 0b11);
        let t_pinned = ThreadControlBlock::new(11, 20, b"pinned", PriorityClass::Normal, 0b01);

        rq0.enqueue(t_pinned);
        rq0.enqueue(t_shareable);

        // CPU 1 steals from CPU 0
        let stolen = rq1.steal_work_from(&mut rq0);
        assert!(stolen.is_some());
        let st = stolen.unwrap();
        assert_eq!(st.tid, 10); // The shareable thread was stolen
        assert_eq!(rq0.len(), 1);
        assert_eq!(rq1.len(), 0);

        // CPU 1 tries to steal again, but only the pinned thread remains
        let second_steal = rq1.steal_work_from(&mut rq0);
        assert!(second_steal.is_none());
    }

    #[test]
    fn test_sleep_queue_deadlines() {
        let mut sq = SleepQueue::new();
        let t1 = ThreadControlBlock::new(100, 50, b"timer1", PriorityClass::Interactive, 0x1);
        let t2 = ThreadControlBlock::new(101, 50, b"timer2", PriorityClass::Normal, 0x1);

        sq.sleep_until(t1, 5_000_000);  // 5ms
        sq.sleep_until(t2, 10_000_000); // 10ms

        assert_eq!(sq.earliest_deadline_ns(), Some(5_000_000));

        let mut woken_count = 0;
        sq.wake_expired(6_000_000, |_t| {
            woken_count += 1;
        });

        assert_eq!(woken_count, 1);
        assert_eq!(sq.earliest_deadline_ns(), Some(10_000_000));
    }
}
