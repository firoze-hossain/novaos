#!/bin/sh
# Month 1 - Foundation: host-side tests for Preemptive Tickless SMP Scheduler with Priority Classes
# 1. 5 Priority classes (Realtime, Interactive, Normal, Batch, Idle).
# 2. Windows NT-style dynamic priority boosting for interactive UI/wakeups and gradual decay.
# 3. Linux CFS-style virtual runtime (vruntime) progression and fair queuing.
# 4. ThreadControlBlock with affinity masks supporting 2 to 32 cores and XSAVE state.
# 5. TicketSpinLock FIFO mutual exclusion without starvation.
# 6. Per-CPU multi-level priority runqueues with O(1) bitmap scheduling.
# 7. Adaptive sleep queues with exact tickless deadline tracking.
# 8. Cross-core work-stealing balancing load while honoring thread affinity.
# 9. Tickless Local APIC one-shot timers eliminating timer interrupt overhead on idle cores.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_sched_tests.sh      (or: make sched-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/sched/thread.rs host tests (TCB, Priority Classes & XSAVE) ---"
    rustc --edition 2021 --test -o "$OUT/thread_test" kernel/src/sched/thread.rs
    "$OUT/thread_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/thread_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/sched/thread.rs tests succeeded."

    echo "--- kernel/src/sched/runqueue.rs host tests (Per-CPU RunQueues & Work-Stealing) ---"
    rustc --edition 2021 --test -o "$OUT/runqueue_test" kernel/src/sched/runqueue.rs
    "$OUT/runqueue_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/runqueue_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/sched/runqueue.rs tests succeeded."

    echo "--- kernel/src/sched/scheduler.rs host tests (SMP Tickless Scheduler) ---"
    rustc --edition 2021 --test -o "$OUT/scheduler_test" kernel/src/sched/scheduler.rs
    "$OUT/scheduler_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/scheduler_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/sched/scheduler.rs tests succeeded."

    echo "[PASS] All Preemptive Tickless SMP Scheduler host tests succeeded."
else
    echo "sched-test: rustc not found - skipping the Rust suite"
fi
