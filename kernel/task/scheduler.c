/*
 * scheduler.c - round-robin preemptive scheduler
 *
 * Phase 57: rewritten for real SMP. Through Phase 56 this file assumed
 * exactly one CPU would ever call into it, so a single shared
 * `current`/`current_index` pair was enough. With a second CPU
 * (Phase 56's AP) genuinely able to run its own process at the same
 * physical instant, that single pair became a real, silent-corruption
 * race - see current[]'s own comment below for the fix.
 */
#include "scheduler.h"
#include "../arch/x86/cpu/tss.h"
#include "../arch/x86/mm/paging.h"
#include "../include/smp.h"
#include "../lib/spinlock.h"
#include "../include/kernel.h"

extern void switch_context(uint32_t* old_esp_out, uint32_t new_esp);

/* Phase 57: one "currently running process" slot per schedulable CPU,
 * replacing the single shared `current` this file had through Phase
 * 56 - two CPUs genuinely running two different processes at the same
 * physical instant each need their own slot; one shared pointer would
 * have the second CPU's context switch silently overwrite the first
 * CPU's idea of what it's running. Indexed by
 * rust_smp_current_cpu_index() everywhere - see smp.h. */
static process_t* current[SCHED_MAX_CPUS];

/* Phase 57: round-robin search position, now shared across every CPU
 * (there is only one process_table[], not one per CPU) so two CPUs
 * scanning at once fairly interleave through it rather than each
 * independently restarting from their own last pick. Protected by
 * scheduler_lock below, same as every other access to shared
 * scheduler state. */
static int search_cursor = -1;

/* Phase 57: guards current[] and search_cursor - the scheduler's own
 * shared state, now genuinely reachable from two CPUs at once. NEVER
 * held across switch_context(): switch_context() only "returns" once
 * some *other* switch_context() call picks this exact saved context
 * to resume - if that resuming call happened while this CPU still
 * held scheduler_lock (a spinlock isn't released by going to sleep),
 * no other CPU could ever acquire this lock again, a real deadlock.
 * Every function below releases the lock before it switches. */
static spinlock_t scheduler_lock;

/* Phase 57: one throwaway "old esp" landing spot per CPU - see
 * scheduler_start()'s own comment for why the value written here is
 * never read again; kept per-CPU (not a single shared throwaway) only
 * because scheduler_start() (BSP) and scheduler_ap_join() (each AP)
 * can genuinely run concurrently with each other during boot. */
static uint32_t startup_esp[SCHED_MAX_CPUS];

void scheduler_add(process_t* p) {
    (void)p; /* nothing to do yet - see the header comment */
}

/* Caller must already hold scheduler_lock. `for_ap` is true for any
 * non-BSP caller (see do_schedule()/scheduler_ap_join() below) and
 * skips any bsp_only process - currently just idle (see process_t's
 * own comment in process.h) - since an AP picking it up would hlt
 * forever waiting for a timer interrupt that's only ever routed to
 * the BSP. */
static process_t* pick_next_locked(bool for_ap) {
    for (int i = 1; i <= MAX_PROCESSES; i++) {
        int idx = (search_cursor + i) % MAX_PROCESSES;
        if (idx < 0) {
            idx += MAX_PROCESSES;
        }
        process_t* p = process_table_entry(idx);
        if (p != NULL &&
            (p->state == PROCESS_READY || p->state == PROCESS_RUNNING)) {
            if (for_ap && p->bsp_only) {
                continue;
            }
            search_cursor = idx;
            return p;
        }
    }
    return NULL;
}

/* Shared preempt/yield path for both scheduler_on_tick() and
 * scheduler_yield(), now parameterized by which CPU is calling -
 * see current[]'s own comment for why that matters. A cpu_index this
 * scheduler doesn't recognize (>= SCHED_MAX_CPUS - e.g.
 * rust_smp_current_cpu_index()'s 0xFF sentinel, widened) is a no-op:
 * fail safe rather than index out of bounds. */
static void do_schedule(uint8_t cpu_index) {
    if (cpu_index >= SCHED_MAX_CPUS) {
        return;
    }

    uint32_t flags = spinlock_acquire(&scheduler_lock);

    process_t* prev = current[cpu_index];
    if (prev == NULL) {
        spinlock_release(&scheduler_lock, flags);
        return; /* this CPU hasn't started scheduling yet */
    }

    process_t* next = pick_next_locked(cpu_index != 0);
    if (next == NULL) {
        /* Nothing eligible at all - shouldn't happen, the idle task
         * never terminates, but fail safe rather than switch into
         * garbage. */
        spinlock_release(&scheduler_lock, flags);
        return;
    }

    if (next == prev) {
        prev->state = PROCESS_RUNNING; /* nothing else ready; keep going */
        spinlock_release(&scheduler_lock, flags);
        return;
    }

    if (prev->state == PROCESS_RUNNING) {
        prev->state = PROCESS_READY;
    }
    next->state = PROCESS_RUNNING;
    current[cpu_index] = next;

    /* Released before switch_context() - see scheduler_lock's own
     * comment for why holding it across a context switch would
     * deadlock every other CPU. */
    spinlock_release(&scheduler_lock, flags);

    tss_set_kernel_stack(cpu_index, next->kernel_stack_top);
    paging_switch_address_space(next->page_directory_phys);
    switch_context(&prev->esp, next->esp);
    /* Execution only reaches here once `prev` is chosen to run again
     * by some future switch_context() call - i.e. this line "returns"
     * an arbitrary number of scheduler ticks later, quite normal for
     * this kind of switch, and not necessarily on this same CPU. */
}

void scheduler_start(void) {
    uint32_t flags = spinlock_acquire(&scheduler_lock);
    process_t* first = pick_next_locked(false);
    if (first == NULL) {
        spinlock_release(&scheduler_lock, flags);
        kernel_panic("scheduler_start: no processes to run");
    }

    current[0] = first;
    first->state = PROCESS_RUNNING;
    spinlock_release(&scheduler_lock, flags);

    tss_set_kernel_stack(0, first->kernel_stack_top);
    paging_switch_address_space(first->page_directory_phys);

    switch_context(&startup_esp[0], first->esp);
    /* Never returns: the boot stack this call happened on is now
     * permanently abandoned. */
}

void scheduler_ap_join(uint8_t cpu_index) {
    if (cpu_index >= SCHED_MAX_CPUS) {
        /* Shouldn't happen - rust_smp_current_cpu_index() only ever
         * hands this AP an index already validated against
         * SCHED_MAX_CPUS at registration time (see apic.rs) - but
         * fail safe: a CPU this scheduler can't track must never run
         * a process at all rather than touch current[]/scheduler_lock
         * out of bounds. */
        for (;;) {
            __asm__ volatile ("hlt");
        }
    }

    process_t* first = NULL;
    for (;;) {
        uint32_t flags = spinlock_acquire(&scheduler_lock);
        first = pick_next_locked(true);
        if (first != NULL) {
            current[cpu_index] = first;
            first->state = PROCESS_RUNNING;
            spinlock_release(&scheduler_lock, flags);
            break;
        }
        spinlock_release(&scheduler_lock, flags);

        /* Nothing schedulable yet - kernel_late_init() (which brings
         * this AP up) runs before any process_create_*() call, see
         * kernel/init/main.c, so this can genuinely happen right
         * after boot. Unlike the BSP's own idle task, this AP has no
         * interrupt that will ever wake it back up (the timer tick
         * stays BSP-only - see process_t's own bsp_only comment), so
         * busy-spin and retry rather than ever halting. */
        __asm__ volatile ("pause" ::: "memory");
    }

    tss_set_kernel_stack(cpu_index, first->kernel_stack_top);
    paging_switch_address_space(first->page_directory_phys);

    switch_context(&startup_esp[cpu_index], first->esp);
    /* Never returns - same reasoning as scheduler_start(). */
}

void scheduler_on_tick(void) {
    do_schedule(rust_smp_current_cpu_index());
}

void scheduler_yield(void) {
    uint8_t cpu = rust_smp_current_cpu_index();
    do_schedule(cpu);

    /* Phase 60 CI-hang fix: do_schedule() no-ops in one tick-of-time
     * for the exact case documented at its own "this CPU hasn't
     * started scheduling yet" early return - true for every
     * scheduler_yield() call made from kernel/boot context (e.g.
     * arp_resolve()/dns_resolve()/tftp_get()'s own wait loops, called
     * from main.c's self-test sequence before scheduler_start() has
     * ever run). Before this fix, that made scheduler_yield() a pure
     * no-op there: those wait loops' only other work per iteration is
     * net_poll(), which - for whichever NIC is attached - means real
     * PCI I/O port reads/writes, each trapped and emulated by the
     * hypervisor. With nothing to slow the loop down, it re-polls as
     * fast as the CPU can retire instructions: hundreds of thousands
     * of iterations, each paying that same trap cost, before the
     * timer can even advance the handful of ticks the loop is
     * actually waiting for. Under real hardware or a KVM-accelerated
     * VM this is wasteful but survivable; under plain (TCG, no-KVM)
     * QEMU - exactly what a GitHub Actions runner uses, having no
     * nested-virtualization support - each trapped I/O access costs
     * enough wall-clock time that a nominal "3 real seconds" wait (a
     * mere 300 ticks) measured in guest time stretched past a full
     * CI test run's timeout budget with the loop never once reaching
     * its own deadline check as satisfied - not a logic bug, an
     * emulation-speed one, but a genuine hang from the outside.
     *
     * The fix: once it's established there's no real scheduling to do
     * (current[cpu] still NULL), actually wait for the next interrupt
     * - hlt - instead of immediately re-entering the caller's loop.
     * The timer IRQ (100Hz) or a NIC RX IRQ both wake this CPU right
     * back up, so a real reply is noticed just as fast as before;
     * what changes is that an idle iteration costs one halted CPU
     * doing nothing instead of a busy-spin hammering hardware
     * registers. Gated on IF actually being set (never assumed): a
     * `hlt` with interrupts disabled never wakes on its own, and
     * while every call this can reach is already documented as
     * IF=1-only (current[cpu] is only ever NULL pre-scheduler-start,
     * a context that unconditionally runs with interrupts enabled -
     * see kernel_late_init()'s own "sti right at the end" comment),
     * checking directly costs nothing and removes the need to trust
     * that invariant never changes underneath this function. */
    if (cpu < SCHED_MAX_CPUS && current[cpu] == NULL) {
        uint32_t eflags;
        __asm__ volatile ("pushf\n\tpop %0" : "=r"(eflags) : : "memory");
        if (eflags & 0x200u) {
            __asm__ volatile ("hlt");
        }
    }
}

process_t* scheduler_current(void) {
    uint8_t cpu = rust_smp_current_cpu_index();
    if (cpu >= SCHED_MAX_CPUS) {
        return NULL;
    }
    return current[cpu];
}