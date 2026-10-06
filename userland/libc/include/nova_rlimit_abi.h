#ifndef NOVA_RLIMIT_ABI_H
#define NOVA_RLIMIT_ABI_H

/*
 * nova_rlimit_abi.h - Phase 86: the ABI of NovaOS's per-process resource
 * limits (SYS_RLIMIT), shared verbatim by the kernel (kernel/task/rlimit.c,
 * rlimit_policy.c) and userland (novasys.h, rlimtest). Dependency-free:
 * plain `unsigned int` / `int` (32 bits on this i686 target - checked by the
 * size assertion at the bottom).
 *
 * WHY THIS EXISTS
 *
 * One runaway process could starve the whole machine and nothing could stop
 * it: a loop that never yields held its CPU forever, a loop that keeps
 * calling sbrk() ate every free frame, and a fork loop filled the process
 * table. (A process that faults is no better: the page-fault handler logs
 * and panics, so today a process leaves only by calling SYS_EXIT itself.)
 * Each limit below is enforced by the part of the kernel that actually
 * decides the thing, not checked after the fact:
 *
 *   MAX_VM_PAGES   the most user pages the process may have mapped (heap,
 *                  stack, image, framebuffer and shared-memory mappings all
 *                  count). Checked on the heap-growth path, BEFORE any page
 *                  is mapped, so a refused sbrk() grows nothing; malloc()
 *                  then returns NULL as it would on any exhausted machine.
 *   CPU_PERCENT    a CAP, not a guarantee: at most this percentage of ONE
 *                  CPU in each 1-second window. A process that has used its
 *                  share is not scheduled again until the window ends (the
 *                  scheduler skips it), so a spin loop is slowed to a crawl
 *                  while everything else keeps its CPU.
 *   CPU_TIME_TICKS the most total CPU time (100Hz ticks) the process may
 *                  consume; the next tick after it is exceeded TERMINATES the
 *                  process with exit code NOVA_RLIMIT_EXIT_CPU. This is how a
 *                  genuinely runaway loop is stopped.
 *   MAX_PROCS      the most live processes that may be created beneath this
 *                  one (children, grandchildren, ...), counted from the
 *                  moment the limit is set. fork() and spawn fail beyond it,
 *                  which is what stops a fork bomb. It is a property of the
 *                  process that SETS it (the "quota root"); descendants
 *                  inherit membership, not the number, so a bomb cannot
 *                  multiply its own allowance by forking.
 *
 * 0 always means "no limit". Limits are inherited by fork() and spawn
 * (except MAX_PROCS, as above); the usage counters start at zero in a child.
 *
 * WHO MAY CHANGE WHAT. Any process may TIGHTEN its own limits (set a limit
 * where there was none, or lower one). Loosening - raising a limit or
 * removing it - and touching ANOTHER process's limits require root (uid 0).
 * A non-root process confined under someone's MAX_PROCS cannot set its own
 * MAX_PROCS at all (that would let it start a fresh allowance). Reading
 * another process's limits and usage is root-only too.
 *
 * Every call returns 0 on success and a NEGATIVE errno on failure.
 */

#define NOVA_SYS_RLIMIT 70 /* EBX = nova_rlimit_t* (in/out) */

#define NOVA_RLIMIT_GET 1u
#define NOVA_RLIMIT_SET 2u

/* SET: which of the four limits to change. */
#define NOVA_RLIMIT_MASK_VM        1u
#define NOVA_RLIMIT_MASK_CPU_PCT   2u
#define NOVA_RLIMIT_MASK_CPU_TIME  4u
#define NOVA_RLIMIT_MASK_PROCS     8u
#define NOVA_RLIMIT_MASK_ALL       15u

#define NOVA_RLIMIT_WINDOW_TICKS   100u /* the CPU-percent window: 1 second */
#define NOVA_RLIMIT_TICK_HZ        100u
#define NOVA_RLIMIT_PAGE_SIZE      4096u

/* killed_by / exit codes */
#define NOVA_RLIMIT_KILL_NONE      0
#define NOVA_RLIMIT_KILL_CPU       1
/* The exit code a process killed for exceeding CPU_TIME_TICKS reports to
 * wait(): negative, so it can never be mistaken for a program's own code. */
#define NOVA_RLIMIT_EXIT_CPU       (-1001)

/* GET fills the limits and the usage; SET reads `mask` and the limits named
 * by it (the usage fields are ignored). `pid` 0 means the caller. */
typedef struct {
    unsigned int op;
    int pid;
    unsigned int mask;
    /* the limits (0 = none) */
    unsigned int max_vm_pages;
    unsigned int cpu_percent;
    unsigned int cpu_time_ticks;
    unsigned int max_procs;
    /* usage, filled by GET */
    unsigned int vm_pages;        /* user pages mapped right now */
    unsigned int vm_peak_pages;   /* the most seen at a check */
    unsigned int cpu_ticks;       /* CPU time consumed, in ticks */
    unsigned int throttled_ticks; /* ticks spent unschedulable for want of quota */
    unsigned int throttle_events; /* windows in which the quota ran out */
    unsigned int throttled;       /* 1 if throttled right now */
    unsigned int procs;           /* live processes beneath this one (if it is a quota root) */
    unsigned int mem_denied;      /* heap growths refused by MAX_VM_PAGES */
    unsigned int now_tick;        /* the kernel clock, for measuring */
    unsigned int tick_hz;
    int quota_root;               /* pid whose MAX_PROCS this process counts against, or 0 */
    unsigned int ap_ticks;        /* times the SECOND CPU's tick IPI enforced this process (evidence that path ran) */
    unsigned int ap_parks;        /* times a throttled process on the SECOND CPU had to wait there for want of anything else to run */
} nova_rlimit_t;

typedef char nova_rlimit_abi_check[(sizeof(nova_rlimit_t) == 80) ? 1 : -1];

/* Error numbers (the kernel cannot include <errno.h>; these repeat the
 * values it returns). */
#define NOVA_RLIMIT_ERR_PERM   1
#define NOVA_RLIMIT_ERR_SRCH   3
#define NOVA_RLIMIT_ERR_FAULT 14
#define NOVA_RLIMIT_ERR_INVAL 22

#endif
