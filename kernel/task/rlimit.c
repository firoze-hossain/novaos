/*
 * kernel/task/rlimit.c - Phase 86: the kernel half of per-process resource
 * limits. The rules and arithmetic are in rlimit_policy.c (pure, tested on
 * the host); this file decides WHEN to ask them and acts on the answers:
 *
 *   - every timer tick, the BSP releases processes whose throttle window
 *     has ended and enforces CPU limits on the process running on EACH CPU;
 *   - the second CPU has no timer interrupt, so for a process that has a CPU
 *     limit and is running there, the BSP's tick sends an IPI
 *     (rlimit_ipi_stub.asm) and the AP enforces its own limits in
 *     rlimit_ipi_handler(). A process WITHOUT a CPU limit never gets this
 *     tick, so unlimited processes behave exactly as they always did;
 *   - memory is checked on the heap-growth path (process_sbrk), and the
 *     process-count limit at fork/spawn - both at the moment of the request,
 *     where refusing is clean;
 *   - SYS_RLIMIT reads and changes limits.
 *
 * KILLING A PROCESS. Until this phase a process could only ever leave by
 * calling SYS_EXIT itself (a faulting process panics the kernel). A CPU-time
 * limit needs the kernel to end a process that will never call anything, so
 * enforce_current() calls process_exit_current() from the tick, ON THE KILLED
 * PROCESS'S OWN KERNEL STACK (which is where a tick that interrupts user code
 * is running). That is only done when the interrupt hit USER mode: the
 * process then provably holds no kernel lock, and abandoning the interrupt
 * frame is no different from abandoning a syscall frame. If the tick landed
 * in kernel mode the kill simply waits for the next tick that lands in user
 * mode.
 */
#include "rlimit.h"
#include "process.h"
#include "scheduler.h"
#include "../arch/x86/cpu/gdt.h"
#include "../arch/x86/cpu/idt.h"
#include "../arch/x86/cpu/isr.h"
#include "../arch/x86/cpu/syscall.h"
#include "../arch/x86/mm/paging.h"
#include "../drivers/timer/timer.h"
#include "../include/kernel.h"
#include "../include/smp.h"
#include "../lib/spinlock.h"
#include "../lib/string.h"
#include "../../userland/libc/include/nova_rlimit_abi.h"

/* The syscall number is defined once, in the shared ABI header, and repeated
 * in kernel/arch/x86/cpu/syscall.h; this stops them drifting apart. */
typedef char rlimit_sysno_check[(SYS_RLIMIT == NOVA_SYS_RLIMIT) ? 1 : -1];
typedef char rlimit_window_check[(RL_WINDOW_TICKS == NOVA_RLIMIT_WINDOW_TICKS) ? 1 : -1];

/* The IDT vector of the per-CPU limit tick: clear of the exceptions (0-31),
 * the IRQs (32-47), the syscall (0x80) and the spurious vector (0xFF). */
#define RLIMIT_IPI_VECTOR 0xF1

extern void rlimit_ipi_stub(void);
extern void rust_apic_send_eoi(void);

/* ------------------------------------------------------------------
 * Helpers.
 * ------------------------------------------------------------------ */

static bool is_live_state(process_state_t s) {
    return s != PROCESS_UNUSED && s != PROCESS_TERMINATED;
}

/* How many live processes currently count against quota root `root_pid`?
 * Counted by scanning the process table EVERY time rather than kept in a
 * counter: a counter would have to be decremented exactly once per exit on
 * every path (and would drift whenever an intermediate parent is reaped),
 * whereas a scan cannot be wrong - the table is the truth. The table is
 * small and this runs only when a process forks or is asked about. */
static uint32_t count_members(int root_pid) {
    uint32_t n = 0;
    int capacity = process_table_capacity();
    for (int i = 0; i < capacity; i++) {
        process_t* q = process_table_entry(i);
        if (q != NULL && is_live_state(q->state) && q->rl.quota_root == root_pid) {
            n++;
        }
    }
    return n;
}

/* May `caller` create another process? (The process-count limit.) Called
 * BEFORE any slot is claimed, so a refused fork costs nothing - claiming a
 * slot and then failing would leak it, and a fork bomb would drain the
 * process table just by being refused.
 *
 * Two members of one group forking at the very same instant on different
 * CPUs can both pass this check and overshoot the cap by one; that is
 * accepted and documented rather than serialised with a global lock on every
 * fork. */
bool rlimit_may_spawn(int caller_pid) {
    process_t* caller = process_find_live(caller_pid);
    if (caller == NULL) {
        return true;
    }
    int32_t root_pid = rl_child_quota_root(&caller->rl, caller->pid);
    if (root_pid == 0) {
        return true;
    }
    process_t* root = process_find_live(root_pid);
    if (root == NULL) {
        return true; /* the process that set the cap has gone: it lapses */
    }
    return rl_spawn_allowed(&root->rl, count_members(root_pid));
}

/* The child has been created (its slot is claimed, its address space built)
 * and is about to be published: it joins its parent's quota group HERE.
 *
 * rlimit_may_spawn() above is the cheap early refusal. It cannot be the whole
 * answer: two members of one group forking at the same instant on different
 * CPUs can both pass it and both create a child, exceeding the cap - and a
 * cap that two CPUs can step over together does not stop a fork bomb on a
 * machine whose point is having two CPUs. So the decision is made again here,
 * under a lock, with the child already counted: the first committer sees
 * itself as the Nth member and passes, the second sees itself as the N+1st
 * and is refused. A refusal here costs the process-table slot (like any
 * failure after allocate_slot()); the early check makes that rare.
 *
 * Returns false if the child must be abandoned. */
static spinlock_t spawn_commit_lock;

bool rlimit_commit_spawn(process_t* child, process_t* parent) {
    int32_t root_pid = rl_child_quota_root(&parent->rl, parent->pid);
    if (root_pid == 0) {
        rl_inherit(&child->rl, &parent->rl, parent->pid);
        return true;
    }
    uint32_t flags = spinlock_acquire(&spawn_commit_lock);
    rl_inherit(&child->rl, &parent->rl, parent->pid); /* now it counts as a member */
    process_t* root = process_find_live(root_pid);
    bool ok = true;
    if (root != NULL && !rl_spawn_allowed(&root->rl, count_members(root_pid) - 1)) {
        child->rl.quota_root = 0; /* abandoned: it must not count */
        ok = false;
    }
    spinlock_release(&spawn_commit_lock, flags);
    return ok;
}

/* ------------------------------------------------------------------
 * Enforcement, from the tick.
 * ------------------------------------------------------------------ */

static void enforce_current(process_t* p, uint32_t now, bool from_user) {
    int v = rl_verdict(&p->rl, now);
    if (v == RL_VERDICT_KILL_CPU) {
        if (!from_user) {
            return; /* see the file header: only ever kill from user mode */
        }
        p->rl.killed_by = NOVA_RLIMIT_KILL_CPU;
        kernel_log("[ .. ] rlimit: pid %d '%s' used %d ticks of CPU (limit "
                   "%d): terminated\n", p->pid, p->name,
                   (int)rl_cpu_total(&p->rl, now), (int)p->rl.cpu_time_ticks);
        process_exit_current(NOVA_RLIMIT_EXIT_CPU); /* never returns */
    } else if (v == RL_VERDICT_THROTTLE) {
        rl_throttle(&p->rl, now);
        scheduler_force_resched(); /* it is skipped from now until its window ends */
    }
}

/* The LAPIC's physical base, from the architectural MSR (the same page the
 * Rust APIC code uses; it is mapped at its physical address). */
static uint32_t lapic_base(void) {
    uint32_t lo, hi;
    __asm__ volatile ("rdmsr" : "=a"(lo), "=d"(hi) : "c"(0x1Bu));
    return lo & 0xFFFFF000u;
}

/* Sends the limit-tick vector to every CPU except this one. */
static void send_tick_ipi_to_others(void) {
    volatile uint32_t* icr_low = (volatile uint32_t*)(lapic_base() + 0x300);
    volatile uint32_t* icr_high = (volatile uint32_t*)(lapic_base() + 0x310);
    *icr_high = 0;
    /* fixed delivery, level assert, destination shorthand 3 = all excluding self */
    *icr_low = RLIMIT_IPI_VECTOR | (1u << 14) | (3u << 18);
    for (uint32_t spins = 0; spins < 100000u; spins++) {
        if ((*icr_low & (1u << 12)) == 0) {
            break; /* delivered */
        }
    }
}

void rlimit_bsp_tick(void) {
    uint32_t now = timer_get_ticks();
    bool released = false;
    bool kick_others = false;
    uint8_t me = rust_smp_current_cpu_index();

    /* release anything whose throttle window has ended */
    int capacity = process_table_capacity();
    for (int i = 0; i < capacity; i++) {
        process_t* q = process_table_entry(i);
        if (q != NULL && is_live_state(q->state) && q->rl.throttled) {
            if (rl_tick_window(&q->rl, now)) {
                released = true;
            }
        }
    }

    /* enforce CPU limits on whatever is running on each CPU */
    for (uint8_t cpu = 0; cpu < SCHED_MAX_CPUS; cpu++) {
        process_t* q = scheduler_cpu_current(cpu);
        if (q == NULL || q->state != PROCESS_RUNNING || !rl_needs_tick(&q->rl)) {
            continue;
        }
        if (cpu == me) {
            enforce_current(q, now, timer_tick_was_user());
        } else {
            kick_others = true;
        }
    }
    if (kick_others) {
        send_tick_ipi_to_others();
    }
    if (released) {
        scheduler_force_resched(); /* do not wait up to a slice for it to be picked */
    }
}

/* The limit tick on a CPU with no timer. Does for that CPU's current
 * process what rlimit_bsp_tick() does for the BSP's. */
void rlimit_ipi_handler(registers_t* regs) {
    rust_apic_send_eoi(); /* first: if we kill the process there is no returning */
    process_t* p = scheduler_current();
    if (p == NULL || p->state != PROCESS_RUNNING || !rl_needs_tick(&p->rl)) {
        return;
    }
    p->rl.ap_ticks++;
    enforce_current(p, timer_get_ticks(), (regs->cs & 3) == 3);

    /* If the process was just throttled and this CPU has nothing else it may
     * run, it cannot idle (only the BSP has an idle task and this CPU has no
     * timer to wake it), so it waits here, in the interrupt, until the BSP's
     * tick ends the window - offering the CPU to anything that becomes
     * runnable meanwhile. The process itself executes no instruction of its
     * own during this time, which is the whole point of the cap. */
    bool parked = false;
    while (p->rl.throttled && scheduler_current() == p) {
        scheduler_force_resched();
        if (p->rl.throttled && scheduler_current() == p) {
            if (!parked) {
                parked = true;
                p->rl.ap_parks++; /* evidence this path ran: see rlimtest */
            }
            __asm__ volatile ("pause" ::: "memory");
        }
    }
}

/* ------------------------------------------------------------------
 * The syscall.
 * ------------------------------------------------------------------ */

int rlimit_sys(int caller_pid, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_rlimit_t), true)) {
        return -NOVA_RLIMIT_ERR_FAULT;
    }
    nova_rlimit_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);

    process_t* caller = process_find_live(caller_pid);
    if (caller == NULL) {
        return -NOVA_RLIMIT_ERR_SRCH;
    }
    int target_pid = (req.pid == 0) ? caller->pid : req.pid;
    bool self = (target_pid == caller->pid);
    /* Privilege BEFORE existence: a non-root caller learns nothing about
     * other processes, not even whether they exist. */
    if (!self && caller->uid != 0) {
        return -NOVA_RLIMIT_ERR_PERM;
    }
    process_t* t = self ? caller : process_find_live(target_pid);
    if (t == NULL) {
        return -NOVA_RLIMIT_ERR_SRCH;
    }

    if (req.op == NOVA_RLIMIT_SET) {
        int rc = rl_check_set(caller->uid, self, &t->rl, req.mask,
                              req.max_vm_pages, req.cpu_percent,
                              req.cpu_time_ticks, req.max_procs);
        if (rc != 0) {
            return rc;
        }
        rl_apply_set(&t->rl, req.mask, req.max_vm_pages, req.cpu_percent,
                     req.cpu_time_ticks, req.max_procs);
        return 0;
    }
    if (req.op != NOVA_RLIMIT_GET) {
        return -NOVA_RLIMIT_ERR_INVAL;
    }

    uint32_t now = timer_get_ticks();
    uint32_t pages = paging_count_user_pages(t->page_directory_phys);
    rl_note_vm(&t->rl, pages);
    req.max_vm_pages = t->rl.max_vm_pages;
    req.cpu_percent = t->rl.cpu_percent;
    req.cpu_time_ticks = t->rl.cpu_time_ticks;
    req.max_procs = t->rl.max_procs;
    req.vm_pages = pages;
    req.vm_peak_pages = t->rl.vm_peak_pages;
    req.cpu_ticks = rl_cpu_total(&t->rl, now);
    req.throttled_ticks = t->rl.throttled_ticks;
    if (t->rl.throttled) {
        req.throttled_ticks += now - t->rl.throttle_began; /* the throttle in progress */
    }
    req.throttle_events = t->rl.throttle_events;
    req.throttled = t->rl.throttled ? 1 : 0;
    req.procs = (t->rl.max_procs != 0) ? count_members(t->pid) : 0;
    req.mem_denied = t->rl.mem_denied;
    req.now_tick = now;
    req.tick_hz = NOVA_RLIMIT_TICK_HZ;
    req.quota_root = t->rl.quota_root;
    req.ap_ticks = t->rl.ap_ticks;
    req.ap_parks = t->rl.ap_parks;
    memcpy((void*)user_ptr, &req, sizeof req);
    return 0;
}

void rlimit_init(void) {
    idt_set_gate(RLIMIT_IPI_VECTOR, (uint32_t)rlimit_ipi_stub, GDT_KERNEL_CODE, 0x8E);
    bool hooked = timer_add_tick_listener(rlimit_bsp_tick);
    kernel_log("[ %s ] Resource limits: memory pages, CPU share (a cap per "
               "%d-tick window), CPU time, process count; the second CPU is "
               "ticked by IPI vector 0x%x while a limited process runs on it\n",
               hooked ? "OK" : "FAIL", (int)RL_WINDOW_TICKS,
               (int)RLIMIT_IPI_VECTOR);
}
