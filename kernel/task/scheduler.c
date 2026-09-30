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

extern void switch_context(uint32_t* old_esp_out, uint32_t new_esp,
                           volatile uint32_t* done_flag);

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
 * the BSP.
 *
 * A real, severe SMP race, found and fixed here: PROCESS_RUNNING is a
 * valid pick target - genuinely needed for the single-runnable-
 * process case (nothing else is PROCESS_READY, so the currently
 * running process has to be able to "pick itself again" to keep
 * going rather than do_schedule() finding nothing at all) - but this
 * function used to accept *any* PROCESS_RUNNING process found by the
 * round-robin search, not specifically the calling CPU's own current
 * one. With two real CPUs (Phase 57's own SMP rewrite - see this
 * file's own header comment) genuinely calling this at the same
 * physical instant, a process actively running on CPU 0 is still
 * PROCESS_RUNNING for the entire time CPU 1's own, completely
 * independent do_schedule() call is scanning the table - so CPU 1
 * could, and directly confirmed did, pick that same process as its
 * own "next" and load its stale, long-since-consumed saved esp (the
 * process's own esp field is only ever updated when *it itself* is
 * the outgoing prev being switched away from - not while it's
 * actively running and reusing that same stack memory for its own,
 * completely unrelated current work, such as a long kernel_log() call
 * whose own local buffer happens to overlap that stale address).
 * Two CPUs then executing on the identical kernel stack at once is
 * real, catastrophic corruption - confirmed directly as the actual
 * explanation for this project's own long-tracked, seemingly random
 * scheduling-corruption bug (PROGRESS.md), not theorized: the
 * corrupted "eflags" value read back at the crash site decoded
 * exactly to ASCII bytes from an in-flight log message string, at the
 * exact stack offset kernel_log()'s own local buffer would occupy.
 * `current[cpu_index]` (Phase 57's own per-CPU "who's running where"
 * state) is exactly the information needed to close this: a
 * PROCESS_RUNNING candidate is only ever a valid pick when it's
 * genuinely *this calling CPU's own* current process, never another
 * CPU's. */
static process_t* pick_next_locked(bool for_ap, uint8_t cpu_index) {
    /* Phase 75: the process table itself can now grow past
     * PROCESS_TABLE_CHUNK_SIZE (see process.h) - read its CURRENT
     * capacity once per call, not a fixed compile-time constant,
     * exactly like process.c's own internal scans already do. A
     * table that grows mid-scan (another CPU calling exec() and
     * triggering allocate_slot()'s own growth) is not a hazard this
     * needs to account for: capacity only ever increases, never
     * shrinks, so at worst this call's own scan simply doesn't
     * consider a slot that became available a moment after it read
     * `capacity` - exactly the same benign, eventually-consistent
     * staleness every other unlocked read of this table already
     * tolerated before this phase. */
    int capacity = process_table_capacity();
    if (capacity <= 0) {
        /* Provably unreachable: process_init() kernel_panic()s if it
         * can't allocate the table's first chunk, and that runs
         * before the scheduler or any process exists - so this
         * function is never called before capacity is at least
         * PROCESS_TABLE_CHUNK_SIZE. Checked anyway rather than relying
         * on that invariant to protect the modulo below from becoming
         * a division by zero - a crash here is the scheduler's own
         * hot path, not a place to trust an invariant silently. */
        return NULL;
    }
    for (int i = 1; i <= capacity; i++) {
        int idx = (search_cursor + i) % capacity;
        if (idx < 0) {
            idx += capacity;
        }
        process_t* p = process_table_entry(idx);
        if (p == NULL) {
            continue;
        }
        /* Phase 73: READY is not enough - see process_t.off_cpu. */
        bool eligible = (p->state == PROCESS_READY && p->off_cpu) ||
                        (p->state == PROCESS_RUNNING &&
                         cpu_index < SCHED_MAX_CPUS &&
                         p == current[cpu_index]);
        if (eligible) {
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

    process_t* next = pick_next_locked(cpu_index != 0, cpu_index);
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
    next->off_cpu = 0; /* on a CPU again until it is switched away from */
    current[cpu_index] = next;

    /* Released before switch_context() - a second CPU spinning on
     * this same lock must not be blocked for the entire duration of
     * a context switch. But this specific release must NOT restore
     * interrupts on THIS CPU yet - that is the real, confirmed bug
     * this exact sequence used to have. spinlock_release()'s own
     * conditional `sti` (when `flags` had IF=1, i.e. whenever
     * do_schedule() was reached via a voluntary scheduler_yield()
     * call rather than a timer tick already running with IF=0) would
     * re-enable interrupts *before* switch_context() below had
     * actually saved `prev`'s own context - a real, unguarded window
     * where a timer tick firing right here recursively re-enters
     * do_schedule() and picks yet another task to run, on top of a
     * `prev` whose own state was never properly saved yet, corrupting
     * it. spinlock_release_no_restore() releases only the lock's own
     * atomic state, touching no interrupt flag at all; `flags` is
     * restored explicitly below, only once switch_context() actually
     * returns - which happens on `prev`'s own resumption, an
     * arbitrary number of scheduler ticks later, exactly the point
     * where it is finally safe to let this CPU's interrupts come back
     * to whatever they were before this specific call into
     * do_schedule() began. */
    spinlock_release_no_restore(&scheduler_lock);

    tss_set_kernel_stack(cpu_index, next->kernel_stack_top);
    paging_switch_address_space(next->page_directory_phys);
    /* This project's own, long-tracked "random" scheduling-corruption
     * bug (PROGRESS.md) was root-caused right here, at this exact call
     * - not inside switch_context() itself, and not from a genuine
     * stack overflow. Two temporary diagnostic checks lived in this
     * exact spot during that investigation (this CPU's own live
     * eflags, and next's own saved eflags at *next->esp, both checked
     * for an unexpectedly-set Trap Flag immediately before this call)
     * and together found the real, confirmed cause: pick_next_locked()
     * above (see its own, extensive comment) could pick a process that
     * was genuinely, actively running on a *different* CPU at this
     * same physical instant, loading that process's stale, already-
     * consumed saved esp - corrupting whichever process actually owned
     * it, from two CPUs executing the identical kernel stack at once.
     * Fixed there; removed here once confirmed, rather than carrying
     * two extra checks (plus a full process-table stack dump) on this
     * kernel's own hottest, most frequently-executed path forever. */
    switch_context(&prev->esp, next->esp, &prev->off_cpu);
    /* Execution only reaches here once `prev` is chosen to run again
     * by some future switch_context() call - i.e. this line "returns"
     * an arbitrary number of scheduler ticks later, quite normal for
     * this kind of switch, and not necessarily on this same CPU. */
    if (flags & 0x200u) {
        __asm__ volatile ("sti" ::: "memory");
    }
}

void scheduler_start(void) {
    uint32_t flags = spinlock_acquire(&scheduler_lock);
    process_t* first = pick_next_locked(false, 0);
    if (first == NULL) {
        spinlock_release(&scheduler_lock, flags);
        kernel_panic("scheduler_start: no processes to run");
    }

    current[0] = first;
    first->state = PROCESS_RUNNING;
    /* Same real bug, same fix, as do_schedule()'s own identical
     * sequence - see that function's own comment for the full
     * account. No manual restore needed after switch_context() here
     * specifically: this call never returns (the boot stack it runs
     * on is abandoned), and the *new* task's own initial, fake stack
     * frame (built by process_create_*()) already has its own eflags
     * baked in with IF=1 - switch_context()'s own popfd for `first`
     * re-enables interrupts correctly as part of its normal restore,
     * once it actually runs, without this function needing to do
     * anything further. */
    spinlock_release_no_restore(&scheduler_lock);

    tss_set_kernel_stack(0, first->kernel_stack_top);
    paging_switch_address_space(first->page_directory_phys);

    first->off_cpu = 0;
    switch_context(&startup_esp[0], first->esp, NULL);
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
        first = pick_next_locked(true, cpu_index);
        if (first != NULL) {
            current[cpu_index] = first;
            first->state = PROCESS_RUNNING;
            /* Same fix as do_schedule()/scheduler_start() - see
             * do_schedule()'s own comment for the full account. */
            spinlock_release_no_restore(&scheduler_lock);
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

    first->off_cpu = 0;
    switch_context(&startup_esp[cpu_index], first->esp, NULL);
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