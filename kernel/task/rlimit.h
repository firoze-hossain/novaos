#ifndef KERNEL_TASK_RLIMIT_H
#define KERNEL_TASK_RLIMIT_H

/*
 * Phase 86: per-process resource limits - memory, CPU share, CPU time and
 * process count - enforced by the scheduler tick and the heap-growth path.
 * The contract is userland/libc/include/nova_rlimit_abi.h.
 *
 * Two layers, deliberately separate:
 *   rlimit_policy.c  PURE rules and arithmetic over rlimit_state_t - the
 *                    window accounting, the throttle/kill verdicts, who may
 *                    change what, the page-count check. No kernel
 *                    dependencies, so tools/tests/rlimit_test.c compiles it
 *                    on the host and attacks it directly.
 *   rlimit.c         the kernel integration: the timer tick, the AP tick
 *                    IPI, killing a process from interrupt context, the
 *                    syscall.
 */

#ifdef RLIMIT_HOST_TEST
#include <stdbool.h>
#include <stdint.h>
#else
#include "../include/types.h"
#endif

#define RL_WINDOW_TICKS 100u

#define RL_VERDICT_OK       0
#define RL_VERDICT_THROTTLE 1
#define RL_VERDICT_KILL_CPU 2

#define RL_MASK_VM       1u
#define RL_MASK_CPU_PCT  2u
#define RL_MASK_CPU_TIME 4u
#define RL_MASK_PROCS    8u
#define RL_MASK_ALL      15u

#define RL_EPERM  1
#define RL_EINVAL 22

typedef struct {
    /* the limits (0 = none) */
    uint32_t max_vm_pages;
    uint32_t cpu_percent;
    uint32_t cpu_time_ticks;
    uint32_t max_procs;
    /* CPU accounting. The process is charged when it is switched OUT, from
     * the tick it was switched IN; in between, usage is computed on demand. */
    uint32_t cpu_ticks;
    uint32_t slice_start;
    bool     on_cpu;
    /* the throttle window */
    uint32_t win_start;
    uint32_t win_used;
    bool     win_valid;
    volatile uint32_t throttled;
    uint32_t throttle_began;
    uint32_t throttled_ticks;
    uint32_t throttle_events;
    /* memory */
    uint32_t vm_peak_pages;
    uint32_t mem_denied;
    /* process count */
    int32_t  quota_root;
    int32_t  killed_by;
    /* how many times the second CPU's tick IPI looked at this process: the
     * evidence, readable through SYS_RLIMIT, that enforcement on a CPU with no
     * timer really happens */
    uint32_t ap_ticks;
    uint32_t ap_parks;
} rlimit_state_t;

/* ---- policy (rlimit_policy.c) ---- */
void rl_reset(rlimit_state_t* rl);
void rl_inherit(rlimit_state_t* child, const rlimit_state_t* parent, int32_t parent_pid);
bool rl_needs_tick(const rlimit_state_t* rl);
void rl_switch_in(rlimit_state_t* rl, uint32_t now);
void rl_switch_out(rlimit_state_t* rl, uint32_t now);
uint32_t rl_cpu_total(const rlimit_state_t* rl, uint32_t now);
uint32_t rl_window_used(const rlimit_state_t* rl, uint32_t now);
uint32_t rl_budget(const rlimit_state_t* rl);
bool rl_tick_window(rlimit_state_t* rl, uint32_t now);
int rl_verdict(const rlimit_state_t* rl, uint32_t now);
void rl_throttle(rlimit_state_t* rl, uint32_t now);
int rl_check_set(uint32_t caller_uid, bool self, const rlimit_state_t* cur,
                 uint32_t mask, uint32_t vm, uint32_t pct, uint32_t cpu_time,
                 uint32_t procs);
void rl_apply_set(rlimit_state_t* rl, uint32_t mask, uint32_t vm, uint32_t pct,
                  uint32_t cpu_time, uint32_t procs);
bool rl_vm_allows(const rlimit_state_t* rl, uint32_t current_pages, uint32_t extra_pages);
void rl_note_vm(rlimit_state_t* rl, uint32_t pages);
int32_t rl_child_quota_root(const rlimit_state_t* parent, int32_t parent_pid);
bool rl_spawn_allowed(const rlimit_state_t* root, uint32_t live_members);

/* ---- kernel integration (rlimit.c) ---- */
void rlimit_init(void);
void rlimit_bsp_tick(void);
int rlimit_sys(int pid, uint32_t user_ptr);
bool rlimit_may_spawn(int caller_pid);

struct process;
bool rlimit_commit_spawn(struct process* child, struct process* parent);

#endif
