/*
 * kernel/task/rlimit_policy.c - Phase 86: the PURE half of per-process
 * resource limits. Everything here is arithmetic and rules over an
 * rlimit_state_t: no kernel calls, no globals, so tools/tests/rlimit_test.c
 * can compile this file on the host (-DRLIMIT_HOST_TEST) and attack it
 * directly. The kernel half (kernel/task/rlimit.c) only decides WHEN to ask
 * these questions and what to do with the answers.
 *
 * Time is the kernel's 100Hz tick counter, a uint32_t that wraps (after
 * ~497 days). Every comparison of two times is therefore written as a
 * signed difference, never as `a < b`.
 *
 * CPU ACCOUNTING. A process is not ticked individually. It is charged when
 * it is switched OUT, for the interval since it was switched IN
 * (rl_switch_in / rl_switch_out, called by the scheduler on every context
 * switch on either CPU). That is unbiased - a process that runs for less
 * than a tick is charged a tick with probability equal to its fraction of
 * one - and costs two subtractions per switch. While a process is running,
 * its usage is computed on demand from the time it was switched in, so a
 * limit can be checked at any tick without waiting for it to be switched out.
 *
 * THE THROTTLE WINDOW. CPU_PERCENT is a cap per RL_WINDOW_TICKS (one
 * second): the budget is window * percent / 100 ticks. A process that has
 * used its budget is marked throttled (the scheduler skips it) until the
 * window ends, when everything resets. The window is anchored the first time
 * the process is switched in and advances in whole windows from there, in
 * O(1) however long the process was idle.
 */
#include "rlimit.h"

static inline int32_t rl_diff(uint32_t a, uint32_t b) {
    return (int32_t)(a - b);
}

void rl_reset(rlimit_state_t* rl) {
    rl->max_vm_pages = 0;
    rl->cpu_percent = 0;
    rl->cpu_time_ticks = 0;
    rl->max_procs = 0;
    rl->cpu_ticks = 0;
    rl->slice_start = 0;
    rl->on_cpu = false;
    rl->win_start = 0;
    rl->win_used = 0;
    rl->win_valid = false;
    rl->throttled = 0;
    rl->throttle_began = 0;
    rl->throttled_ticks = 0;
    rl->throttle_events = 0;
    rl->vm_peak_pages = 0;
    rl->mem_denied = 0;
    rl->quota_root = 0;
    rl->killed_by = 0;
    rl->ap_ticks = 0;
    rl->ap_parks = 0;
}

/* The pid whose MAX_PROCS a child of `parent` counts against: the parent
 * itself if the parent set a MAX_PROCS (it is a quota root), else whatever
 * the parent already counts against. */
int32_t rl_child_quota_root(const rlimit_state_t* parent, int32_t parent_pid) {
    return parent->max_procs != 0 ? parent_pid : parent->quota_root;
}

/* A child starts with the parent's LIMITS (memory, CPU share, CPU time) and
 * empty USAGE. MAX_PROCS is deliberately not copied: it belongs to the
 * process that set it, and descendants inherit MEMBERSHIP in its quota
 * (quota_root), not the number - otherwise every process in a fork bomb
 * would carry its own fresh allowance. */
void rl_inherit(rlimit_state_t* child, const rlimit_state_t* parent, int32_t parent_pid) {
    rl_reset(child);
    child->max_vm_pages = parent->max_vm_pages;
    child->cpu_percent = parent->cpu_percent;
    child->cpu_time_ticks = parent->cpu_time_ticks;
    child->quota_root = rl_child_quota_root(parent, parent_pid);
}

/* Do CPU limits need the tick to look at this process while it runs? (The
 * memory and process-count limits are enforced at the moment of the request
 * and need no tick.) */
bool rl_needs_tick(const rlimit_state_t* rl) {
    return rl->cpu_percent != 0 || rl->cpu_time_ticks != 0;
}

/* Advances the window to the one containing `now`, in O(1). Returns true if
 * it moved (the process, if throttled, is released). */
static bool rl_roll(rlimit_state_t* rl, uint32_t now) {
    uint32_t elapsed = now - rl->win_start;
    if (rl_diff(now, rl->win_start) < (int32_t)RL_WINDOW_TICKS) {
        return false;
    }
    rl->win_start += (elapsed / RL_WINDOW_TICKS) * RL_WINDOW_TICKS;
    rl->win_used = 0;
    if (rl->throttled) {
        rl->throttled_ticks += now - rl->throttle_began;
        rl->throttled = 0;
    }
    return true;
}

/* Charges the CPU time the process used over [from, now). */
static void rl_charge(rlimit_state_t* rl, uint32_t from, uint32_t now) {
    if (rl_diff(now, from) <= 0) {
        return;
    }
    rl->cpu_ticks += now - from;
    if (rl->win_valid) {
        (void)rl_roll(rl, now);
        /* the part of the interval before the (possibly new) window began
         * belongs to an earlier window and is not charged to this one */
        if (rl_diff(from, rl->win_start) < 0) {
            from = rl->win_start;
        }
        if (rl_diff(now, from) > 0) {
            rl->win_used += now - from;
        }
    }
}

void rl_switch_in(rlimit_state_t* rl, uint32_t now) {
    if (!rl->win_valid) {
        rl->win_start = now;
        rl->win_used = 0;
        rl->win_valid = true;
    }
    rl->slice_start = now;
    rl->on_cpu = true;
}

void rl_switch_out(rlimit_state_t* rl, uint32_t now) {
    if (!rl->on_cpu) {
        return;
    }
    rl_charge(rl, rl->slice_start, now);
    rl->on_cpu = false;
}

/* Total CPU time as of `now`, including the slice in progress. */
uint32_t rl_cpu_total(const rlimit_state_t* rl, uint32_t now) {
    uint32_t total = rl->cpu_ticks;
    if (rl->on_cpu && rl_diff(now, rl->slice_start) > 0) {
        total += now - rl->slice_start;
    }
    return total;
}

/* CPU time used in the window containing `now`, as of `now`, without
 * changing anything. */
uint32_t rl_window_used(const rlimit_state_t* rl, uint32_t now) {
    uint32_t ws = rl->win_start;
    uint32_t used = rl->win_used;
    if (!rl->win_valid) {
        return 0;
    }
    if (rl_diff(now, ws) >= (int32_t)RL_WINDOW_TICKS) {
        ws += ((now - ws) / RL_WINDOW_TICKS) * RL_WINDOW_TICKS;
        used = 0;
    }
    if (rl->on_cpu) {
        uint32_t from = rl->slice_start;
        if (rl_diff(from, ws) < 0) {
            from = ws;
        }
        if (rl_diff(now, from) > 0) {
            used += now - from;
        }
    }
    return used;
}

/* A nonzero cap must never mean "no CPU at all": with at least 100 ticks in
 * the window, one percent is at least one tick. (Checked at compile time
 * rather than guarded at run time: a guard for a case that cannot occur only
 * suggests that it can.) */
typedef char rl_window_is_at_least_100_ticks[(RL_WINDOW_TICKS >= 100u) ? 1 : -1];

/* The CPU ticks per window a CPU_PERCENT cap allows; 0 if there is no cap. */
uint32_t rl_budget(const rlimit_state_t* rl) {
    if (rl->cpu_percent == 0) {
        return 0;
    }
    return (RL_WINDOW_TICKS * rl->cpu_percent) / 100u;
}

/* Called every tick for the process running at `now`. */
int rl_verdict(const rlimit_state_t* rl, uint32_t now) {
    if (rl->cpu_time_ticks != 0 && rl_cpu_total(rl, now) >= rl->cpu_time_ticks) {
        return RL_VERDICT_KILL_CPU;
    }
    if (rl->cpu_percent != 0 && !rl->throttled && rl_window_used(rl, now) >= rl_budget(rl)) {
        return RL_VERDICT_THROTTLE;
    }
    return RL_VERDICT_OK;
}

void rl_throttle(rlimit_state_t* rl, uint32_t now) {
    rl->throttled = 1;
    rl->throttle_began = now;
    rl->throttle_events++;
}

/* Called every tick for every process that is throttled: releases it when its
 * window has ended. Returns true if it was released. */
bool rl_tick_window(rlimit_state_t* rl, uint32_t now) {
    bool was_throttled = rl->throttled != 0;
    if (!rl->win_valid) {
        return false;
    }
    (void)rl_roll(rl, now);
    return was_throttled && !rl->throttled;
}

/* Loosening: raising a limit, or removing one. (Setting a limit where there
 * was none, or lowering one, is tightening.) */
static bool loosens(uint32_t cur, uint32_t requested) {
    return cur != 0 && (requested == 0 || requested > cur);
}

/* May the caller make this change? 0, or -errno.
 *
 *  - EINVAL: an empty or unknown mask, or a CPU percentage above 100.
 *  - Another process's limits: root only.
 *  - Loosening any limit: root only. Tightening your own is always allowed.
 *  - MAX_PROCS: a process already counted against someone's MAX_PROCS cannot
 *    set its own (that would hand it a fresh allowance and let it escape the
 *    cap it was confined under); root may. */
int rl_check_set(uint32_t caller_uid, bool self, const rlimit_state_t* cur,
                 uint32_t mask, uint32_t vm, uint32_t pct, uint32_t cpu_time,
                 uint32_t procs) {
    if (mask == 0 || (mask & ~RL_MASK_ALL) != 0) {
        return -RL_EINVAL;
    }
    if ((mask & RL_MASK_CPU_PCT) != 0 && pct > 100) {
        return -RL_EINVAL;
    }
    if (!self && caller_uid != 0) {
        return -RL_EPERM;
    }
    if (caller_uid == 0) {
        return 0;
    }
    if ((mask & RL_MASK_VM) != 0 && loosens(cur->max_vm_pages, vm)) {
        return -RL_EPERM;
    }
    if ((mask & RL_MASK_CPU_PCT) != 0 && loosens(cur->cpu_percent, pct)) {
        return -RL_EPERM;
    }
    if ((mask & RL_MASK_CPU_TIME) != 0 && loosens(cur->cpu_time_ticks, cpu_time)) {
        return -RL_EPERM;
    }
    if ((mask & RL_MASK_PROCS) != 0) {
        if (cur->quota_root != 0 || loosens(cur->max_procs, procs)) {
            return -RL_EPERM;
        }
    }
    return 0;
}

void rl_apply_set(rlimit_state_t* rl, uint32_t mask, uint32_t vm, uint32_t pct,
                  uint32_t cpu_time, uint32_t procs) {
    if (mask & RL_MASK_VM) {
        rl->max_vm_pages = vm;
    }
    if (mask & RL_MASK_CPU_PCT) {
        rl->cpu_percent = pct;
        if (pct == 0 && rl->throttled) {
            /* removing the cap must not leave a process stranded until its
             * window happens to end */
            rl->throttled_ticks += 0;
            rl->throttled = 0;
        }
    }
    if (mask & RL_MASK_CPU_TIME) {
        rl->cpu_time_ticks = cpu_time;
    }
    if (mask & RL_MASK_PROCS) {
        rl->max_procs = procs;
    }
}

/* May the process grow by `extra_pages` when it has `current_pages` mapped?
 * Written so a huge request cannot overflow its way past the limit, and so a
 * process already over a limit tightened after the fact can still shrink or
 * stand still (extra == 0) but not grow. */
bool rl_vm_allows(const rlimit_state_t* rl, uint32_t current_pages, uint32_t extra_pages) {
    if (extra_pages == 0 || rl->max_vm_pages == 0) {
        return true;
    }
    if (current_pages >= rl->max_vm_pages) {
        return false;
    }
    return extra_pages <= rl->max_vm_pages - current_pages;
}

void rl_note_vm(rlimit_state_t* rl, uint32_t pages) {
    if (pages > rl->vm_peak_pages) {
        rl->vm_peak_pages = pages;
    }
}

/* May a new process be created beneath this quota root, given how many
 * processes already live beneath it? */
bool rl_spawn_allowed(const rlimit_state_t* root, uint32_t live_members) {
    return root->max_procs == 0 || live_members < root->max_procs;
}
