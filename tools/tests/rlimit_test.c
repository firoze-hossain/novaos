/*
 * rlimit_test.c - Phase 86: host-side tests for kernel/task/rlimit_policy.c,
 * the pure rules and arithmetic behind per-process resource limits.
 *
 * The policy is small but it is exactly the kind of code where a mistake is
 * invisible in a quick run: an off-by-one at a budget boundary, a charge that
 * lands in the wrong window, a comparison that breaks when the 100Hz tick
 * counter wraps. So the evidence is layered:
 *
 *  1. BOUNDARIES. Every threshold is checked at, one below and one above.
 *  2. THE WINDOW ARITHMETIC against a deliberately naive REFERENCE MODEL: an
 *     array with one entry per tick saying whether the process ran. A random
 *     sequence of switch-in, switch-out and window ticks is applied to both;
 *     after EVERY step the O(1) arithmetic must agree with counting entries.
 *  3. A SIMULATION of the scheduler loop: a CPU-hungry process capped at 20%
 *     must get EXACTLY 20 ticks in every one of fifty windows, and a process
 *     with a CPU-time limit must be condemned on exactly the tick it reaches it.
 *  4. WRAPAROUND. The same scenarios started 30 ticks before the 32-bit tick
 *     counter wraps.
 *  5. PRIVILEGE and INHERITANCE as tables, and a fork-bomb simulation proving
 *     the process-count rule neither lets a bomb exceed its cap nor drifts
 *     (it counts live members each time, there is no counter to get wrong).
 *
 * Build: see tools/tests/run_rlimit_tests.sh (also under ASan/UBSan).
 */
#define RLIMIT_HOST_TEST 1
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "../../kernel/task/rlimit_policy.c"

static long checks, failures;
#define CHECK(cond, ...) do { checks++; if (!(cond)) { failures++; \
    if (failures <= 20) { printf("FAIL %s:%d: %s\n   ", __FILE__, __LINE__, #cond); \
    printf(__VA_ARGS__); printf("\n"); } } } while (0)

#define W RL_WINDOW_TICKS

static uint64_t rng_state = 0x9E3779B97F4A7C15ull;
static uint32_t rnd(void) {
    rng_state ^= rng_state << 13;
    rng_state ^= rng_state >> 7;
    rng_state ^= rng_state << 17;
    return (uint32_t)(rng_state >> 11);
}

/* ---- 1. boundaries ----------------------------------------------------------- */

static void test_budget_and_verdicts(void) {
    rlimit_state_t rl;
    rl_reset(&rl);
    CHECK(rl_budget(&rl) == 0, "no cap, no budget");
    CHECK(rl_verdict(&rl, 1000) == RL_VERDICT_OK, "an unlimited process is never condemned");
    CHECK(!rl_needs_tick(&rl), "an unlimited process needs no tick");
    uint32_t pcts[] = {1, 2, 20, 50, 99, 100};
    uint32_t want[] = {1, 2, 20, 50, 99, 100};
    for (int i = 0; i < 6; i++) {
        rl.cpu_percent = pcts[i];
        CHECK(rl_budget(&rl) == want[i], "%u%% of a %u-tick window is %u ticks, got %u", pcts[i], W, want[i], rl_budget(&rl));
    }
    CHECK(rl_needs_tick(&rl), "a CPU cap needs the tick");
    /* a CPU-TIME limit needs it too: without the tick a runaway loop is never looked at */
    rl_reset(&rl);
    rl.cpu_time_ticks = 5;
    CHECK(rl_needs_tick(&rl), "a CPU-time limit needs the tick (it is how a runaway loop gets stopped)");
    rl.cpu_time_ticks = 0;
    rl.max_vm_pages = 10; rl.max_procs = 3;
    CHECK(!rl_needs_tick(&rl), "memory and process-count limits are enforced at the request, not by the tick");
    rl_reset(&rl);

    /* the throttle trips exactly when the window's use REACHES the budget */
    rl_reset(&rl);
    rl.cpu_percent = 20;
    rl_switch_in(&rl, 1000);
    CHECK(rl_verdict(&rl, 1000 + 19) == RL_VERDICT_OK, "19 of 20: still within budget");
    CHECK(rl_verdict(&rl, 1000 + 20) == RL_VERDICT_THROTTLE, "20 of 20: the budget is spent");
    CHECK(rl_verdict(&rl, 1000 + 21) == RL_VERDICT_THROTTLE, "21 of 20");
    rl_throttle(&rl, 1020);
    CHECK(rl_verdict(&rl, 1021) == RL_VERDICT_OK, "an already-throttled process is not condemned again");

    /* the CPU-time limit trips exactly when TOTAL use reaches it, and beats the throttle */
    rl_reset(&rl);
    rl.cpu_time_ticks = 50;
    rl_switch_in(&rl, 0);
    CHECK(rl_verdict(&rl, 49) == RL_VERDICT_OK, "49 of 50");
    CHECK(rl_verdict(&rl, 50) == RL_VERDICT_KILL_CPU, "50 of 50: condemned");
    rl.cpu_percent = 10;
    CHECK(rl_verdict(&rl, 50) == RL_VERDICT_KILL_CPU, "when both are exceeded, the kill wins over the throttle");
    rl_switch_out(&rl, 30);
    rl_switch_in(&rl, 60);
    CHECK(rl_cpu_total(&rl, 70) == 40, "total = what was charged (30) + the slice in progress (10)");
    CHECK(rl_verdict(&rl, 80) == RL_VERDICT_KILL_CPU, "30 charged + 20 running = 50");
}

static void test_split_at_a_window_boundary(void) {
    rlimit_state_t rl;
    rl_reset(&rl);
    rl_switch_in(&rl, 1000);          /* the window is anchored at 1000 */
    rl_switch_out(&rl, 1005);         /* ran 5 ticks in window [1000,1100) */
    rl_switch_in(&rl, 1090);
    rl_switch_out(&rl, 1130);         /* 10 ticks in [1000,1100), 30 in [1100,1200) */
    CHECK(rl.cpu_ticks == 45, "total charged %u", rl.cpu_ticks);
    CHECK(rl.win_start == 1100 && rl.win_used == 30, "the 40-tick slice must split 10/30 across the boundary: window %u used %u", rl.win_start, rl.win_used);

    /* a process idle for many windows: the roll is O(1) and exact */
    rl_reset(&rl);
    rl_switch_in(&rl, 5000);
    rl_switch_out(&rl, 5030);
    CHECK(rl_tick_window(&rl, 9000) == false, "nothing throttled, nothing released");
    CHECK(rl.win_start == 9000 && rl.win_used == 0, "40 windows later: window %u used %u", rl.win_start, rl.win_used);
    rl_switch_in(&rl, 9250);          /* not on a window start: 9250 is 250 into the window at 9000 */
    CHECK(rl_window_used(&rl, 9260) == 10, "usage in the NEW window only: %u", rl_window_used(&rl, 9260));
    CHECK(rl_window_used(&rl, 9350) == 0 + 50 + 0 || rl_window_used(&rl, 9350) == 50, "crossing into the next window mid-slice drops the earlier part: %u", rl_window_used(&rl, 9350));
}

static void test_throttle_release_and_accounting(void) {
    rlimit_state_t rl;
    rl_reset(&rl);
    rl.cpu_percent = 20;
    rl_switch_in(&rl, 1000);
    rl_switch_out(&rl, 1020);
    rl_throttle(&rl, 1020);
    CHECK(rl.throttled == 1 && rl.throttle_events == 1, "throttled once");
    CHECK(!rl_tick_window(&rl, 1099), "still inside the window: not released");
    CHECK(rl.throttled == 1, "still throttled at tick 99");
    CHECK(rl_tick_window(&rl, 1100), "released at the window boundary");
    CHECK(rl.throttled == 0 && rl.win_used == 0, "released with a fresh window");
    CHECK(rl.throttled_ticks == 80, "throttled from 1020 to 1100: %u", rl.throttled_ticks);
    CHECK(!rl_tick_window(&rl, 1101), "releasing twice is a no-op");
    CHECK(rl.throttled_ticks == 80, "and does not double-count");
    /* a second throttle accumulates */
    rl_switch_in(&rl, 1100);
    rl_switch_out(&rl, 1120);
    rl_throttle(&rl, 1120);
    rl_tick_window(&rl, 1200);
    CHECK(rl.throttle_events == 2 && rl.throttled_ticks == 160, "two throttles: %u events, %u ticks", rl.throttle_events, rl.throttled_ticks);
}

/* ---- 2. the reference model ---------------------------------------------------------- */

enum { SIM_TICKS = 12000 };

static void model_run(uint32_t t0, uint64_t seed) {
    rng_state = seed;
    rlimit_state_t rl;
    rl_reset(&rl);
    static uint32_t window_done[SIM_TICKS / 1 + 2];
    memset(window_done, 0, sizeof window_done);
    uint32_t total_done = 0;
    bool on = false, anchored = false;
    uint32_t in_at = 0, anchor = 0, now = t0;
    long compared = 0;
    for (int step = 0; step < 30000 && (now - t0) < SIM_TICKS - 8; step++) {
        now += rnd() % 4;
        switch (rnd() % 3) {
        case 0:
            if (!on) {
                rl_switch_in(&rl, now);
                on = true;
                in_at = now;
                if (!anchored) { anchored = true; anchor = now; }
            }
            break;
        case 1:
            if (on) {
                rl_switch_out(&rl, now);
                on = false;
                /* the reference: spread the interval [in_at, now) over the windows it touches */
                for (uint32_t t = in_at; (int32_t)(now - t) > 0; ) {
                    uint32_t k = (t - anchor) / W;
                    uint32_t wend = anchor + (k + 1) * W;
                    uint32_t stop = ((int32_t)(wend - now) < 0) ? wend : now;
                    if ((int32_t)(stop - now) > 0) stop = now;
                    window_done[k] += stop - t;
                    total_done += stop - t;
                    t = stop;
                }
            }
            break;
        default:
            (void)rl_tick_window(&rl, now);
            break;
        }
        uint32_t ref_total = total_done + (on ? now - in_at : 0);
        CHECK(rl_cpu_total(&rl, now) == ref_total, "step %d: total %u, reference %u", step, rl_cpu_total(&rl, now), ref_total);
        if (anchored) {
            uint32_t k = (now - anchor) / W;
            uint32_t ws = anchor + k * W;
            uint32_t ref_win = window_done[k];
            if (on) {
                uint32_t from = ((int32_t)(in_at - ws) < 0) ? ws : in_at;
                ref_win += now - from;
            }
            CHECK(rl_window_used(&rl, now) == ref_win, "step %d (t=%u): window use %u, reference %u", step, now - t0, rl_window_used(&rl, now), ref_win);
        } else {
            CHECK(rl_window_used(&rl, now) == 0, "before the first switch-in nothing has been used");
        }
        compared++;
    }
    CHECK(compared > 5000, "only %ld steps were compared", compared);
}

static void test_random_model(void) {
    model_run(1000, 0x1234ABCDull);
    model_run(0, 0xDEADBEEFull);
    model_run(77777, 0xC0FFEEull);
    /* and across the wrap of the 32-bit tick counter */
    model_run(0xFFFFFFFFu - 2500, 0xBADF00Dull);
    model_run(0xFFFFFFFFu - 40, 0x5EEDull);
}

/* ---- 3. the scheduler loop, simulated -------------------------------------------------- */

/* Each tick: the window ticks, a released process is switched back in, then
 * the verdict for the running process is acted on - exactly the kernel's order. */
static void sim_throttle(uint32_t t0, uint32_t pct, int windows) {
    rlimit_state_t rl;
    rl_reset(&rl);
    rl.cpu_percent = pct;
    bool on = false;
    static uint32_t per_window[200];
    memset(per_window, 0, sizeof per_window);
    for (uint32_t t = 0; t < (uint32_t)windows * W; t++) {
        uint32_t now = t0 + t;
        (void)rl_tick_window(&rl, now);
        if (!on && !rl.throttled) {
            rl_switch_in(&rl, now);
            on = true;
        }
        if (on) {
            if (rl_verdict(&rl, now) == RL_VERDICT_THROTTLE) {
                rl_switch_out(&rl, now);
                rl_throttle(&rl, now);
                on = false;
            } else {
                per_window[t / W]++; /* this tick is spent running */
            }
        }
    }
    uint32_t budget = (W * pct) / 100;
    int exact = 0;
    for (int w = 0; w < windows; w++) {
        if (per_window[w] == budget) exact++;
    }
    CHECK(exact == windows, "%u%%: %d of %d windows got exactly %u ticks (first: %u)", pct, exact, windows, budget, per_window[0]);
    CHECK(rl.throttle_events == (uint32_t)windows || rl.throttle_events == (uint32_t)windows - 1,
          "%u%%: %u throttle events over %d windows", pct, rl.throttle_events, windows);
}

static void test_throttle_simulation(void) {
    uint32_t pcts[] = {1, 10, 20, 50, 75, 99};
    for (int i = 0; i < 6; i++) {
        sim_throttle(5000, pcts[i], 50);
        sim_throttle(0xFFFFFFFFu - 30, pcts[i], 30); /* crossing the counter's wrap in the first window */
    }
    /* a process capped at 100% is never throttled in a way that matters: it gets the whole window */
    rlimit_state_t rl;
    rl_reset(&rl);
    rl.cpu_percent = 100;
    rl_switch_in(&rl, 0);
    CHECK(rl_verdict(&rl, 99) == RL_VERDICT_OK, "100%% is not exhausted at tick 99");
}

static void test_cpu_time_kill_is_exact(void) {
    uint32_t starts[] = {0, 1234, 0xFFFFFFFFu - 20};
    for (int s = 0; s < 3; s++) {
        rlimit_state_t rl;
        rl_reset(&rl);
        rl.cpu_time_ticks = 50;
        rl_switch_in(&rl, starts[s]);
        int killed_at = -1;
        for (int t = 0; t < 200; t++) {
            if (rl_verdict(&rl, starts[s] + (uint32_t)t) == RL_VERDICT_KILL_CPU) { killed_at = t; break; }
        }
        CHECK(killed_at == 50, "start %u: condemned at tick %d, expected exactly 50", starts[s], killed_at);
    }
    /* time spent NOT running is not charged: 30 ticks, a pause, 20 more */
    rlimit_state_t rl;
    rl_reset(&rl);
    rl.cpu_time_ticks = 50;
    rl_switch_in(&rl, 0);
    rl_switch_out(&rl, 30);
    CHECK(rl_verdict(&rl, 5000) == RL_VERDICT_OK, "a long pause costs nothing (30 of 50 used)");
    rl_switch_in(&rl, 5000);
    CHECK(rl_verdict(&rl, 5019) == RL_VERDICT_OK && rl_verdict(&rl, 5020) == RL_VERDICT_KILL_CPU, "30 + 20 = 50");
}

/* ---- 5. privilege, inheritance, memory, process count -------------------------------------- */

static void test_privilege_matrix(void) {
    rlimit_state_t cur;
    rl_reset(&cur);
    /* argument validation applies to everyone, root included */
    CHECK(rl_check_set(0, true, &cur, 0, 0, 0, 0, 0) == -RL_EINVAL, "an empty mask");
    CHECK(rl_check_set(0, true, &cur, 16, 0, 0, 0, 0) == -RL_EINVAL, "an unknown mask bit");
    CHECK(rl_check_set(0, true, &cur, RL_MASK_CPU_PCT, 0, 101, 0, 0) == -RL_EINVAL, "101%%");
    CHECK(rl_check_set(0, true, &cur, RL_MASK_CPU_PCT, 0, 100, 0, 0) == 0, "100%% is fine");
    CHECK(rl_check_set(700, true, &cur, RL_MASK_CPU_PCT, 0, 101, 0, 0) == -RL_EINVAL, "101%% for a non-root too");
    /* nobody but root touches another process */
    CHECK(rl_check_set(700, false, &cur, RL_MASK_VM, 10, 0, 0, 0) == -RL_EPERM, "non-root, another process");
    CHECK(rl_check_set(0, false, &cur, RL_MASK_VM, 10, 0, 0, 0) == 0, "root, another process");
    /* tightening is free; loosening is root's */
    CHECK(rl_check_set(700, true, &cur, RL_MASK_VM, 100, 0, 0, 0) == 0, "no limit -> a limit is tightening");
    cur.max_vm_pages = 100;
    CHECK(rl_check_set(700, true, &cur, RL_MASK_VM, 50, 0, 0, 0) == 0, "100 -> 50");
    CHECK(rl_check_set(700, true, &cur, RL_MASK_VM, 100, 0, 0, 0) == 0, "100 -> 100 changes nothing");
    CHECK(rl_check_set(700, true, &cur, RL_MASK_VM, 101, 0, 0, 0) == -RL_EPERM, "100 -> 101 loosens");
    CHECK(rl_check_set(700, true, &cur, RL_MASK_VM, 0, 0, 0, 0) == -RL_EPERM, "100 -> none loosens");
    CHECK(rl_check_set(0, true, &cur, RL_MASK_VM, 0, 0, 0, 0) == 0, "root may remove it");
    cur.cpu_percent = 30; cur.cpu_time_ticks = 500;
    /* the top of the range: setting the maximum value again is not loosening it */
    rlimit_state_t top;
    rl_reset(&top);
    top.max_vm_pages = 0xFFFFFFFFu;
    CHECK(rl_check_set(700, true, &top, RL_MASK_VM, 0xFFFFFFFFu, 0, 0, 0) == 0, "UINT32_MAX -> UINT32_MAX changes nothing (no wrap)");
    CHECK(rl_check_set(700, true, &top, RL_MASK_VM, 0xFFFFFFFEu, 0, 0, 0) == 0, "UINT32_MAX -> UINT32_MAX-1 tightens");
    CHECK(rl_check_set(700, true, &cur, RL_MASK_CPU_PCT, 0, 31, 0, 0) == -RL_EPERM, "30%% -> 31%%");
    CHECK(rl_check_set(700, true, &cur, RL_MASK_CPU_PCT, 0, 29, 0, 0) == 0, "30%% -> 29%%");
    CHECK(rl_check_set(700, true, &cur, RL_MASK_CPU_TIME, 0, 0, 501, 0) == -RL_EPERM, "500 -> 501 ticks");
    CHECK(rl_check_set(700, true, &cur, RL_MASK_CPU_TIME, 0, 0, 1, 0) == 0, "500 -> 1 tick");
    /* one forbidden field poisons a multi-field request */
    CHECK(rl_check_set(700, true, &cur, RL_MASK_VM | RL_MASK_CPU_PCT, 50, 90, 0, 0) == -RL_EPERM, "one loosening in a pair is refused");
    /* MAX_PROCS: a confined process cannot start a fresh allowance */
    CHECK(rl_check_set(700, true, &cur, RL_MASK_PROCS, 0, 0, 0, 5) == 0, "an unconfined process may set its own");
    cur.quota_root = 42;
    CHECK(rl_check_set(700, true, &cur, RL_MASK_PROCS, 0, 0, 0, 5) == -RL_EPERM, "a confined non-root process may not");
    CHECK(rl_check_set(0, true, &cur, RL_MASK_PROCS, 0, 0, 0, 5) == 0, "root may");
    cur.quota_root = 0; cur.max_procs = 5;
    CHECK(rl_check_set(700, true, &cur, RL_MASK_PROCS, 0, 0, 0, 6) == -RL_EPERM, "5 -> 6 loosens");
    CHECK(rl_check_set(700, true, &cur, RL_MASK_PROCS, 0, 0, 0, 3) == 0, "5 -> 3");

    /* apply touches only what the mask names */
    rl_reset(&cur);
    cur.max_vm_pages = 7; cur.cpu_percent = 8; cur.cpu_time_ticks = 9; cur.max_procs = 10;
    rl_apply_set(&cur, RL_MASK_CPU_PCT | RL_MASK_PROCS, 111, 55, 333, 4);
    CHECK(cur.max_vm_pages == 7 && cur.cpu_percent == 55 && cur.cpu_time_ticks == 9 && cur.max_procs == 4, "apply: %u %u %u %u", cur.max_vm_pages, cur.cpu_percent, cur.cpu_time_ticks, cur.max_procs);
}

static void test_inheritance(void) {
    rlimit_state_t parent, child;
    rl_reset(&parent);
    parent.max_vm_pages = 100; parent.cpu_percent = 25; parent.cpu_time_ticks = 700; parent.max_procs = 8;
    parent.cpu_ticks = 999; parent.throttle_events = 3; parent.mem_denied = 4; parent.vm_peak_pages = 55; parent.throttled = 1;
    rl_inherit(&child, &parent, 17);
    CHECK(child.max_vm_pages == 100 && child.cpu_percent == 25 && child.cpu_time_ticks == 700, "limits are inherited");
    CHECK(child.max_procs == 0, "MAX_PROCS is NOT inherited: it belongs to the process that set it");
    CHECK(child.quota_root == 17, "the child counts against its limited parent (pid 17), got %d", child.quota_root);
    CHECK(child.cpu_ticks == 0 && child.throttle_events == 0 && child.mem_denied == 0 && child.vm_peak_pages == 0 && child.throttled == 0 && !child.on_cpu && !child.win_valid,
          "usage starts at zero (and a throttled parent does not make a throttled child)");
    /* a grandchild still counts against the ORIGINAL root */
    rlimit_state_t grandchild;
    rl_inherit(&grandchild, &child, 18);
    CHECK(grandchild.quota_root == 17, "a grandchild counts against the same root, got %d", grandchild.quota_root);
    /* an unlimited, unconfined parent produces unconfined children */
    rlimit_state_t free_parent, free_child;
    rl_reset(&free_parent);
    rl_inherit(&free_child, &free_parent, 5);
    CHECK(free_child.quota_root == 0, "no limit anywhere above: nobody to count against");
}

static void test_memory_limit(void) {
    rlimit_state_t rl;
    rl_reset(&rl);
    CHECK(rl_vm_allows(&rl, 1000, 1000000), "no limit: anything");
    rl.max_vm_pages = 100;
    CHECK(rl_vm_allows(&rl, 90, 10), "90 + 10 = 100: exactly at the limit is allowed");
    CHECK(!rl_vm_allows(&rl, 90, 11), "90 + 11 = 101: one over");
    CHECK(!rl_vm_allows(&rl, 100, 1), "already at the limit");
    CHECK(rl_vm_allows(&rl, 100, 0), "standing still is always allowed");
    CHECK(rl_vm_allows(&rl, 150, 0), "so is standing still when already over (a limit tightened after the fact)");
    CHECK(!rl_vm_allows(&rl, 150, 1), "but not growing");
    CHECK(!rl_vm_allows(&rl, 0, 0xFFFFFFFFu), "a huge request cannot overflow its way past the limit");
    CHECK(!rl_vm_allows(&rl, 0xFFFFFFF0u, 0x20), "nor can a huge current count wrap");
    CHECK(!rl_vm_allows(&rl, 50, 0xFFFFFFFFu), "nor current + extra wrapping to a small number");
    rl_note_vm(&rl, 40); rl_note_vm(&rl, 90); rl_note_vm(&rl, 60);
    CHECK(rl.vm_peak_pages == 90, "the peak only rises: %u", rl.vm_peak_pages);
}

/* A fork bomb against a quota root, with the counting done the way the kernel does it: by
 * scanning the live processes for members, with no counter anywhere. */
typedef struct { bool live; int pid; rlimit_state_t rl; } sim_proc_t;

static int count_members(const sim_proc_t* t, int n, int root_pid) {
    int c = 0;
    for (int i = 0; i < n; i++) if (t[i].live && t[i].rl.quota_root == root_pid) c++;
    return c;
}

static void test_fork_bomb(void) {
    enum { N = 400 };
    static sim_proc_t t[N];
    memset(t, 0, sizeof t);
    int next_pid = 100;
    /* process 0 is the quota root, capped at 12 */
    t[0].live = true; t[0].pid = next_pid++;
    rl_reset(&t[0].rl);
    t[0].rl.max_procs = 12;
    int denied = 0, spawned = 0, max_live = 0;
    for (int step = 0; step < 20000; step++) {
        int who = (int)(rnd() % N);
        if (!t[who].live) continue;
        if (rnd() % 3 == 0 && who != 0) {                 /* a member exits (the root never does) */
            t[who].live = false;
            continue;
        }
        /* it tries to fork */
        int root = rl_child_quota_root(&t[who].rl, t[who].pid);
        bool ok = true;
        if (root != 0) {
            const sim_proc_t* r = NULL;
            for (int i = 0; i < N; i++) if (t[i].live && t[i].pid == root) r = &t[i];
            if (r != NULL) ok = rl_spawn_allowed(&r->rl, (uint32_t)count_members(t, N, root));
        }
        if (!ok) { denied++; continue; }
        int slot = -1;
        for (int i = 0; i < N; i++) if (!t[i].live) { slot = i; break; }
        if (slot < 0) continue;
        t[slot].live = true;
        t[slot].pid = next_pid++;
        rl_inherit(&t[slot].rl, &t[who].rl, t[who].pid);
        spawned++;
        int live_members = count_members(t, N, t[0].pid);
        if (live_members > max_live) max_live = live_members;
        CHECK(live_members <= 12, "the bomb exceeded its cap: %d live members", live_members);
    }
    CHECK(max_live == 12, "the cap was reached exactly (%d) - the rule is neither too loose nor too tight", max_live);
    CHECK(denied > 100, "the bomb was refused %d times", denied);
    CHECK(spawned > 100, "and members came and went (%d spawns), so any drift would have shown", spawned);
    /* every member, however deep, counts against the one root */
    int deep = 0;
    for (int i = 1; i < N; i++) if (t[i].live) { CHECK(t[i].rl.quota_root == t[0].pid, "member %d counts against %d", i, t[i].rl.quota_root); deep++; }
    CHECK(deep > 0, "some members survive to check");
    /* with room freed, a fork is allowed again */
    for (int i = 1; i < N; i++) t[i].live = false;
    CHECK(rl_spawn_allowed(&t[0].rl, (uint32_t)count_members(t, N, t[0].pid)), "an empty quota allows a spawn");
}

int main(void) {
    test_budget_and_verdicts();
    test_split_at_a_window_boundary();
    test_throttle_release_and_accounting();
    test_random_model();
    test_throttle_simulation();
    test_cpu_time_kill_is_exact();
    test_privilege_matrix();
    test_inheritance();
    test_memory_limit();
    test_fork_bomb();
    printf("rlimit host test: %ld checks, %ld failures\n", checks, failures);
    return failures ? 1 : 0;
}
