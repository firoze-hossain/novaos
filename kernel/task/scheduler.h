#ifndef TASK_SCHEDULER_H
#define TASK_SCHEDULER_H

#include "process.h"
#include "../include/types.h"

/* Registers a newly-created process as eligible to run. Doesn't
 * maintain a separate ready queue - round robin currently just scans
 * the whole process table each time it needs to pick a task (see
 * scheduler.c), which is simple and correct at MAX_PROCESSES=16 but
 * would want a real queue if that ever grows much larger. */
void scheduler_add(process_t* p);

/* BSP-only: picks the first eligible process and switches into it as
 * CPU 0. Called once, after every initial task has been created;
 * never returns. See scheduler_ap_join() below for the AP equivalent. */
void scheduler_start(void);

/* Phase 57: the AP equivalent of scheduler_start(). Called once by
 * each AP - a small C shim off kernel/rust/apic.rs's rust_ap_main(),
 * after that AP has loaded its own TSS (tss_load_this_cpu()) and
 * learned its own scheduler CPU index
 * (rust_smp_current_cpu_index()) - which is exactly the `cpu_index`
 * passed in here. Unlike scheduler_start(), an AP has no guarantee any
 * process exists yet the instant it calls this (kernel_late_init() -
 * which brings every AP up - runs before any process_create_*() call,
 * see kernel/init/main.c) and, more importantly, has no periodic wake
 * source of its own to fall back on the way the BSP's own idle-hlt
 * loop does (the timer tick stays routed to the BSP only - see
 * process_t's own bsp_only comment in process.h) - so this busy-spins,
 * retrying rather than ever halting, until something schedulable
 * actually exists. Never returns. A `cpu_index` this scheduler can't
 * track (>= SCHED_MAX_CPUS - shouldn't happen; see smp.h) halts that
 * AP outright rather than ever touching scheduler state with an
 * out-of-range index. */
void scheduler_ap_join(uint8_t cpu_index);

/* Called from the timer IRQ (see kernel/init/main.c's timer tick
 * hook), on whichever CPU took the interrupt. Preempts that CPU's own
 * current process in favor of the next READY one, round robin, if
 * there is one. A no-op on a CPU rust_smp_current_cpu_index() doesn't
 * recognize. */
void scheduler_on_tick(void);

/* Voluntary reschedule of the calling CPU's own current process -
 * used by process_exit_current() and (later) a SYS_YIELD syscall. */
void scheduler_yield(void);

/* Returns the process currently running on *this* CPU (identified via
 * rust_smp_current_cpu_index()) - NULL if that CPU hasn't started
 * scheduling yet, or isn't one this scheduler recognizes. */
process_t* scheduler_current(void);

#endif
