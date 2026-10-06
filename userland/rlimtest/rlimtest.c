/*
 * rlimtest.c - Phase 86: the in-OS conformance test for per-process resource
 * limits (SYS_RLIMIT). A real ring-3 program against the real kernel, and -
 * because the point is stopping a runaway process from starving the machine -
 * it creates real runaways: processes that spin forever, grow the heap until
 * something stops them, and fork until something stops them.
 * kernel/task/exec_trust_demo.c runs it at boot and tools/python/
 * test_runner.py checks its "[rlimtest] ok:" / "[rlimtest] PASS" lines.
 *
 * It is the in-OS counterpart of tools/tests/rlimit_test.c (which attacks the
 * pure policy on the host): that proves the rules and the arithmetic, this
 * proves the whole stack - the syscall, the scheduler tick, the timer, the
 * second CPU's IPI tick, the heap path, fork, process exit, slot reuse.
 *
 * The scenarios, each a way the feature could be wrong in a way a plain
 * "it returned 0" would miss:
 *
 *  - CPU TIME: three processes set a 400ms limit and then spin in a loop that
 *    never makes a system call. Before this phase nothing could end them (a
 *    process left only by calling SYS_EXIT itself). All three must die with
 *    NOVA_RLIMIT_EXIT_CPU, promptly - and since there are two CPUs and the
 *    second one has no timer interrupt of its own, at least one of them is
 *    typically running there when it is condemned. A control process with no
 *    limit keeps working throughout.
 *  - CPU SHARE: two processes capped at 20% and one unlimited process all spin
 *    for four seconds. The capped ones must get roughly a fifth of a CPU
 *    (measured by the kernel's own accounting), be throttled in several
 *    windows, and be far behind the unlimited one.
 *  - MEMORY: the heap path stops at EXACTLY the limit (not one page over, not
 *    one under), a refused request grows NOTHING, standing still is allowed,
 *    and a loop that calls sbrk() until it fails stops at the limit while the
 *    rest of the machine carries on.
 *  - PROCESS COUNT: a group capped at 4 refuses a 5th member however it is
 *    created (direct child or grandchild), recovers the moment one exits, shows
 *    no drift over twenty exit-and-refork cycles, and a genuine fork bomb
 *    never gets more than 4 members alive - sampled WHILE it runs.
 *  - PRIVILEGE: a process may tighten its own limits but not loosen them, may
 *    not touch another process's, and (confined by someone else's cap) may not
 *    set a process-count limit of its own.
 *  - INHERITANCE and HYGIENE: children inherit the limits but not the usage,
 *    and the thirty processes that reuse the dead hogs' process-table slots
 *    start with no limits, no throttle and no usage at all.
 *
 * Every failure prints "[rlimtest] FAIL: ..." (the uppercase word also trips
 * test_runner.py's global no-fail assertion). Output for passing checks
 * deliberately avoids the words that assertion greps for.
 */
#include <errno.h>
#include <novashm.h>
#include <novasys.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int checks, failures;

#define CHECK(cond, ...) do { \
    checks++; \
    if (!(cond)) { \
        failures++; \
        if (failures <= 25) { \
            printf("[rlimtest] FAIL: %s (line %d) ", #cond, __LINE__); \
            printf(__VA_ARGS__); \
            printf("\n"); \
        } \
    } \
} while (0)

#define EXPECT(call, want) do { \
    int rc_ = (call); \
    checks++; \
    if (rc_ != (want)) { \
        failures++; \
        if (failures <= 25) \
            printf("[rlimtest] FAIL: %s returned %d, expected %d (line %d)\n", \
                   #call, rc_, (int)(want), __LINE__); \
    } \
} while (0)

#define VM   NOVA_RLIMIT_MASK_VM
#define PCT  NOVA_RLIMIT_MASK_CPU_PCT
#define CPU  NOVA_RLIMIT_MASK_CPU_TIME
#define PROC NOVA_RLIMIT_MASK_PROCS

/* ---- helpers --------------------------------------------------------------------- */

static int rl_get(int pid, nova_rlimit_t* out) {
    memset(out, 0, sizeof *out);
    out->op = NOVA_RLIMIT_GET;
    out->pid = pid;
    return sys_rlimit(out);
}

static int rl_set(int pid, unsigned int mask, unsigned int vm, unsigned int pct, unsigned int cpu, unsigned int procs) {
    nova_rlimit_t r;
    memset(&r, 0, sizeof r);
    r.op = NOVA_RLIMIT_SET;
    r.pid = pid;
    r.mask = mask;
    r.max_vm_pages = vm;
    r.cpu_percent = pct;
    r.cpu_time_ticks = cpu;
    r.max_procs = procs;
    return sys_rlimit(&r);
}

static int now_s(void) {
    nova_rtc_time_t t;
    sys_rtc_read(&t);
    return t.hour * 3600 + t.minute * 60 + t.second;
}
static int elapsed_s(int start) {
    int e = now_s() - start;
    return e < 0 ? e + 86400 : e;
}

/* Waits for `pid` to exit, up to `timeout_s`. Returns 1 and sets *code if it
 * did, 0 if it is still running. */
static int wait_exit(int pid, int timeout_s, int* code) {
    int start = now_s();
    for (;;) {
        int rc = sys_wait_nonblock(pid, code);
        if (rc == 0) return 1;
        if (rc == -2) { *code = -99999; return 1; }
        if (elapsed_s(start) > timeout_s) return 0;
        sys_yield();
    }
}

/* A shared page the children report through (it survives fork as a shared
 * mapping - Phase 83). */
static nova_shm_region_t mbr;
static volatile unsigned int* res;
enum { R_A = 0, R_B = 16, R_C = 32, R_GATE = 48, R_COUNT = 64, R_FEED_STOP = R_GATE + 6 };

static void spin_forever(void) {
    volatile unsigned long x = 0;
    for (;;) {
        x++;
    }
}

/* ---- group 1: the API, privilege, inheritance --------------------------------------- */

static void user_child(int parent_pid) {
    failures = 0;
    CHECK(sys_login("persisted", "persisted-pw") == 0, "child: login");
    CHECK(sys_getuid() == 700, "child: uid %u", sys_getuid());
    nova_rlimit_t u;
    EXPECT(rl_get(0, &u), 0);
    /* tightening what is unlimited is allowed; loosening it again is not */
    EXPECT(rl_set(0, VM, 5000, 0, 0, 0), 0);
    EXPECT(rl_set(0, VM, 6000, 0, 0, 0), -EPERM);
    EXPECT(rl_set(0, VM, 0, 0, 0, 0), -EPERM);
    EXPECT(rl_set(0, VM, 4000, 0, 0, 0), 0);
    EXPECT(rl_get(0, &u), 0);
    CHECK(u.max_vm_pages == 4000, "child: limit now %u", u.max_vm_pages);
    EXPECT(rl_set(0, PCT, 0, 30, 0, 0), 0);
    EXPECT(rl_set(0, PCT, 0, 31, 0, 0), -EPERM);
    EXPECT(rl_set(0, PCT, 0, 101, 0, 0), -EINVAL);     /* invalid is invalid for everyone, whatever the privilege */
    EXPECT(rl_set(0, CPU, 0, 0, 900, 0), 0);
    EXPECT(rl_set(0, CPU, 0, 0, 901, 0), -EPERM);
    /* a multi-field request with one loosening in it is refused as a whole */
    EXPECT(rl_set(0, VM | PCT, 3000, 99, 0, 0), -EPERM);
    EXPECT(rl_get(0, &u), 0);
    CHECK(u.max_vm_pages == 4000 && u.cpu_percent == 30, "child: a refused request must change nothing (%u / %u)", u.max_vm_pages, u.cpu_percent);
    /* another process: not even readable, and a non-root caller learns nothing */
    EXPECT(rl_get(parent_pid, &u), -EPERM);
    EXPECT(rl_set(parent_pid, VM, 10, 0, 0, 0), -EPERM);
    EXPECT(rl_get(99999, &u), -EPERM);                 /* EPERM, not ESRCH: it does not say whether pid 99999 exists */
    /* its own process cap: a first one is tightening, raising it is not */
    EXPECT(rl_set(0, PROC, 0, 0, 0, 3), 0);
    EXPECT(rl_set(0, PROC, 0, 0, 0, 4), -EPERM);
    sys_exit(failures);
}

static void test_api_and_privilege(void) {
    int f0 = failures;
    nova_rlimit_t u, u2;
    EXPECT(rl_get(0, &u), 0);
    CHECK(u.max_vm_pages == 0 && u.cpu_percent == 0 && u.cpu_time_ticks == 0 && u.max_procs == 0 && u.quota_root == 0,
          "a fresh process has no limits: %u %u %u %u", u.max_vm_pages, u.cpu_percent, u.cpu_time_ticks, u.max_procs);
    CHECK(u.vm_pages > 0 && u.tick_hz == 100 && u.throttled == 0, "usage: %u pages, %u Hz", u.vm_pages, u.tick_hz);
    EXPECT(rl_get(0, &u2), 0);
    CHECK((int)(u2.now_tick - u.now_tick) >= 0, "the clock must not go backwards");

    /* validation */
    EXPECT(rl_set(0, 0, 1, 1, 1, 1), -EINVAL);
    EXPECT(rl_set(0, 16, 1, 1, 1, 1), -EINVAL);
    EXPECT(rl_set(0, PCT, 0, 101, 0, 0), -EINVAL);
    nova_rlimit_t bad;
    memset(&bad, 0, sizeof bad);
    bad.op = 99;
    EXPECT(sys_rlimit(&bad), -EINVAL);
    EXPECT(rl_get(99999, &u), -ESRCH);                  /* root asking about a pid that does not exist */
    EXPECT(rl_set(99999, VM, 5, 0, 0, 0), -ESRCH);

    /* set / get round trip, then root lifts it again (root may loosen) */
    EXPECT(rl_set(0, VM | PCT | CPU, u.vm_pages + 5000, 40, 100000, 0), 0);
    EXPECT(rl_get(0, &u2), 0);
    CHECK(u2.max_vm_pages == u.vm_pages + 5000 && u2.cpu_percent == 40 && u2.cpu_time_ticks == 100000 && u2.max_procs == 0,
          "round trip: %u %u %u %u", u2.max_vm_pages, u2.cpu_percent, u2.cpu_time_ticks, u2.max_procs);
    EXPECT(rl_set(0, PCT, 0, 0, 0, 0), 0);              /* root removes the CPU cap */
    EXPECT(rl_set(0, VM | CPU, 0, 0, 0, 0), 0);
    EXPECT(rl_get(0, &u2), 0);
    CHECK(u2.max_vm_pages == 0 && u2.cpu_percent == 0 && u2.cpu_time_ticks == 0, "root removed them all");

    /* hostile pointers */
    void* nowhere[] = { (void*)0, (void*)0x00100000, (void*)0x50000000, (void*)0xFFFFFFF8 };
    for (unsigned i = 0; i < sizeof nowhere / sizeof nowhere[0]; i++) {
        EXPECT(sys_rlimit((nova_rlimit_t*)nowhere[i]), -EFAULT);
    }

    /* a different user */
    int me = 0;
    {
        nova_shm_region_t tmp;
        EXPECT(nova_shm_create(&tmp, 4096), 0);
        me = nova_shm_owner_pid(tmp.handle);
        nova_shm_detach(&tmp);
        sys_shm_destroy(tmp.handle);
    }
    CHECK(me > 0, "could not learn my own pid");
    int pid = sys_fork();
    if (pid == 0) {
        user_child(me);
    }
    CHECK(pid > 0, "fork failed: %d", pid);
    int code = -1;
    CHECK(wait_exit(pid, 10, &code), "the uid-700 child never finished");
    CHECK(code == 0, "the uid-700 child reported %d failed checks", code);

    /* inheritance: limits are inherited, usage is not, and the process cap is not copied */
    EXPECT(rl_get(0, &u), 0);
    EXPECT(rl_set(0, VM | CPU, u.vm_pages + 4000, 0, 1000000, 0), 0);
    pid = sys_fork();
    if (pid == 0) {
        failures = 0;
        nova_rlimit_t c;
        EXPECT(rl_get(0, &c), 0);
        CHECK(c.max_vm_pages == u.vm_pages + 4000 && c.cpu_time_ticks == 1000000, "the child must inherit the limits: %u %u", c.max_vm_pages, c.cpu_time_ticks);
        CHECK(c.cpu_ticks < 20 && c.throttle_events == 0 && c.mem_denied == 0 && c.vm_peak_pages <= c.vm_pages + 1, "usage must start at zero: cpu %u events %u", c.cpu_ticks, c.throttle_events);
        CHECK(c.max_procs == 0 && c.quota_root == 0, "no process cap was set above, so none is inherited");
        sys_exit(failures);
    }
    CHECK(pid > 0 && wait_exit(pid, 10, &code) && code == 0, "the inheriting child failed (code %d)", code);
    EXPECT(rl_set(0, VM | CPU, 0, 0, 0, 0), 0);

    if (failures == f0) printf("[rlimtest] ok: API - get/set round trip, validation, hostile pointers, tightening only for non-root, another process off limits, usage starts at zero in a child\n");
}

/* ---- group 2: memory -------------------------------------------------------------------- */

static void memory_child(void) {
    failures = 0;
    nova_rlimit_t u;
    EXPECT(rl_get(0, &u), 0);
    unsigned int p0 = u.vm_pages;
    EXPECT(rl_set(0, VM, p0 + 8, 0, 0, 0), 0);

    /* exactly eight pages, then no more */
    int grown = 0;
    for (int i = 0; i < 8; i++) {
        if (sys_sbrk(4096) != (void*)-1) grown++;
    }
    CHECK(grown == 8, "the first 8 pages were within the limit, but only %d were granted", grown);
    CHECK(sys_sbrk(4096) == (void*)-1, "the 9th page must be refused");
    CHECK(sys_sbrk(1) == (void*)-1, "so must even one byte that needs a new page");
    CHECK(sys_sbrk(0) != (void*)-1, "standing still is always allowed");
    EXPECT(rl_get(0, &u), 0);
    CHECK(u.vm_pages == p0 + 8, "%u pages mapped, expected exactly %u", u.vm_pages, p0 + 8);
    CHECK(u.mem_denied == 2, "two requests were refused (and counted): %u", u.mem_denied);
    CHECK(u.vm_peak_pages >= p0 + 8, "peak %u", u.vm_peak_pages);

    /* a refused request grows NOTHING: ask for 5 pages with only 3 free */
    EXPECT(rl_set(0, VM, p0 + 11, 0, 0, 0), 0);        /* root: loosening */
    CHECK(sys_sbrk(5 * 4096) == (void*)-1, "5 pages with 3 free must be refused");
    EXPECT(rl_get(0, &u), 0);
    CHECK(u.vm_pages == p0 + 8, "a refused sbrk must map NOTHING, but %u pages are mapped (expected %u)", u.vm_pages, p0 + 8);
    CHECK(sys_sbrk(3 * 4096) != (void*)-1, "the 3 that do fit");
    EXPECT(rl_get(0, &u), 0);
    CHECK(u.vm_pages == p0 + 11, "now at the limit: %u vs %u", u.vm_pages, p0 + 11);

    /* the runaway: sbrk until it fails. The limit must stop it, at the limit. */
    EXPECT(rl_set(0, VM, p0 + 1500, 0, 0, 0), 0);
    unsigned int extra = 0;
    while (sys_sbrk(4096) != (void*)-1) {
        extra++;
        if (extra > 5000) break; /* a safety net for the TEST: the limit must stop it long before */
    }
    EXPECT(rl_get(0, &u), 0);
    CHECK(extra == 1500 - 11, "the heap hog was stopped after %u pages, expected exactly %u", extra, 1500 - 11);
    CHECK(u.vm_pages == p0 + 1500, "it holds %u pages, the limit is %u", u.vm_pages, p0 + 1500);

    /* a fork child inherits the SAME ceiling, counted over the whole address space */
    int pid = sys_fork();
    if (pid == 0) {
        failures = 0;
        CHECK(sys_sbrk(4096) == (void*)-1, "a child of a process at its limit starts at its limit");
        sys_exit(failures);
    }
    int code = -1;
    CHECK(pid > 0 && wait_exit(pid, 10, &code) && code == 0, "the inheriting grandchild failed (code %d)", code);
    sys_exit(failures);
}

static void test_memory(void) {
    int f0 = failures;
    int pid = sys_fork();
    if (pid == 0) {
        memory_child();
    }
    CHECK(pid > 0, "fork failed: %d", pid);
    int code = -1;
    CHECK(wait_exit(pid, 20, &code), "the memory test child never finished");
    CHECK(code == 0, "the memory child reported %d failed checks", code);
    /* the machine carries on: the runaway's 6MB went back when it exited */
    void* mine = sys_sbrk(64 * 4096);
    CHECK(mine != (void*)-1, "the machine is out of memory after the hog exited");
    if (failures == f0) printf("[rlimtest] ok: memory - stops at exactly the limit, a refused sbrk maps nothing, a heap-hog loop is halted, children inherit the ceiling\n");
}

/* ---- group 3: CPU time - a runaway loop is ended ------------------------------------------- */

static void test_cpu_time(void) {
    int f0 = failures;
    int control = -1;
    int ap_seen = 0, rounds = 0, all_stopped = 1, codes_ok = 1, last_hog = -1;
    int start = now_s();
    /* The second CPU reschedules only when ITS process yields, so whether a hog
     * ever lands there depends on what that CPU happens to be running. Three
     * ordinary processes that do nothing but yield make it likely that the
     * second CPU has something that keeps asking for more work; they take no
     * part in the checks and are stopped when the group ends. */
    res[R_FEED_STOP] = 0;
    int feeders[3];
    for (int i = 0; i < 3; i++) {
        feeders[i] = sys_fork();
        if (feeders[i] == 0) {
            int t = now_s();
            while (res[R_FEED_STOP] == 0 && elapsed_s(t) < 25) sys_yield();
            sys_exit(0);
        }
    }
    /* Which CPU a process lands on is the scheduler's business, and the second
     * CPU has no timer: a runaway there is stopped ONLY by the IPI tick. So the
     * test runs up to four rounds of three hogs and requires that, in at least
     * one, the kernel's own counter shows the second CPU's tick enforcing one. */
    for (int round = 0; round < 4 && !ap_seen; round++) {
        rounds++;
        int hogs[3], done[3] = {0, 0, 0};
        for (int i = 0; i < 3; i++) {
            hogs[i] = sys_fork();
            if (hogs[i] == 0) {
                EXPECT(rl_set(0, CPU, 0, 0, 40, 0), 0); /* 40 ticks = 400ms of CPU */
                spin_forever();                         /* no system call, ever: nothing else can stop it */
            }
            CHECK(hogs[i] > 0, "fork failed: %d", hogs[i]);
        }
        last_hog = hogs[0];
        if (round == 0) {
            control = sys_fork();
            if (control == 0) {
                /* an ordinary process, no limit, doing ordinary work while the hogs spin */
                int t0 = now_s(), laps = 0;
                while (elapsed_s(t0) < 2) {
                    sys_yield();
                    laps++;
                }
                /* with both CPUs busy it runs in turn, so a modest number of laps in
                 * two seconds is what "not starved" looks like */
                sys_exit(laps > 4 ? 0 : 1);
            }
        }
        int pending = 3, t_round = now_s();
        while (pending > 0 && elapsed_s(t_round) < 12) {
            for (int i = 0; i < 3; i++) {
                if (done[i]) continue;
                int code = 0;
                if (sys_wait_nonblock(hogs[i], &code) == 0) {
                    done[i] = 1;
                    pending--;
                    if (code != NOVA_RLIMIT_EXIT_CPU) {
                        codes_ok = 0;
                        printf("[rlimtest] FAIL: hog %d ended with code %d, expected the CPU-limit code %d\n", i, code, NOVA_RLIMIT_EXIT_CPU);
                    }
                } else {
                    nova_rlimit_t u;
                    if (rl_get(hogs[i], &u) == 0 && u.ap_ticks > 0) ap_seen = 1;
                }
            }
            sys_yield();
        }
        if (pending > 0) all_stopped = 0;
    }
    CHECK(all_stopped, "a runaway loop was never stopped");
    CHECK(codes_ok, "a hog ended with the wrong code");
    CHECK(elapsed_s(start) <= 14, "stopping the hogs took %d seconds", elapsed_s(start));
    CHECK(ap_seen, "in %d rounds no hog was ever enforced by the second CPU's tick - that path was not shown to work", rounds);
    res[R_FEED_STOP] = 1;
    for (int i = 0; i < 3; i++) {
        int fc;
        wait_exit(feeders[i], 10, &fc);
    }
    int ccode = -1;
    CHECK(control > 0 && wait_exit(control, 12, &ccode) && ccode == 0, "the unlimited control process was starved (code %d)", ccode);
    /* the condemned are gone, not zombies the limit forgot */
    nova_rlimit_t u;
    EXPECT(rl_get(last_hog, &u), -ESRCH);
    if (failures == f0) printf("[rlimtest] ok: CPU time - runaway loops that never make a system call were terminated (one by the second CPU's tick IPI), and an ordinary process was not starved\n");
}

/* ---- group 4: CPU share - a cap, measured ------------------------------------------------------- */

static void spinner(unsigned int pct, unsigned int slot) {
    failures = 0;
    nova_rlimit_t u;
    if (pct != 0) {
        EXPECT(rl_set(0, PCT, 0, pct, 0, 0), 0);
    }
    EXPECT(rl_get(0, &u), 0);
    unsigned int end = u.now_tick + 400; /* four seconds of wall clock */
    unsigned int cpu0 = u.cpu_ticks;
    volatile unsigned long x = 0;
    for (;;) {
        for (int j = 0; j < 30000; j++) x++;
        if (rl_get(0, &u) != 0) break;
        if ((int)(u.now_tick - end) >= 0) break;
    }
    res[slot + 0] = u.cpu_ticks - cpu0;
    res[slot + 1] = u.throttle_events;
    res[slot + 2] = u.throttled_ticks;
    res[slot + 3] = u.now_tick - (end - 400);
    res[slot + 4] = u.ap_ticks;
    res[slot + 5] = u.ap_parks;
    sys_exit(failures);
}

static void test_cpu_share(void) {
    int f0 = failures;
    memset((void*)res, 0, 64 * sizeof(unsigned int));
    int a1 = sys_fork();
    if (a1 == 0) spinner(20, R_A);
    int a2 = sys_fork();
    if (a2 == 0) spinner(20, R_B);
    int b = sys_fork();
    if (b == 0) spinner(0, R_C);
    CHECK(a1 > 0 && a2 > 0 && b > 0, "fork failed");
    int c1 = -1, c2 = -1, cb = -1;
    CHECK(wait_exit(a1, 15, &c1) && wait_exit(a2, 15, &c2) && wait_exit(b, 15, &cb), "a spinner never finished");
    CHECK(c1 == 0 && c2 == 0 && cb == 0, "spinner codes %d %d %d", c1, c2, cb);
    printf("[rlimtest] note: over ~%u ticks of wall clock the capped processes got %u and %u CPU ticks (cap 20%%), the unlimited one %u\n",
           res[R_A + 3], res[R_A + 0], res[R_B + 0], res[R_C + 0]);
    printf("[rlimtest] note: windows throttled %u and %u; throttled ticks %u and %u\n", res[R_A + 1], res[R_B + 1], res[R_A + 2], res[R_B + 2]);
    unsigned int wall = res[R_A + 3] ? res[R_A + 3] : 400;
    unsigned int want = wall / 5; /* 20% */
    for (int k = 0; k < 2; k++) {
        unsigned int s = k == 0 ? R_A : R_B;
        CHECK(res[s + 0] >= want * 55 / 100 && res[s + 0] <= want * 150 / 100,
              "capped process %d used %u CPU ticks; 20%% of %u is %u (accepting %u to %u)", k, res[s + 0], wall, want, want * 55 / 100, want * 150 / 100);
        CHECK(res[s + 1] >= 3, "capped process %d was throttled in only %u windows", k, res[s + 1]);
        /* Throttled time is not "all the time it was not running": a released
         * process also waits READY for a CPU (the unlimited spinner holds the
         * second CPU, which has no timer, so the capped ones queue for the
         * BSP in 50ms slices) and that waiting is not throttling. What must
         * hold is that it was throttled for a large part of the time, and
         * that it could never have been throttled while running. */
        CHECK(res[s + 2] >= wall / 4, "capped process %d spent only %u of %u ticks throttled", k, res[s + 2], wall);
        CHECK(res[s + 0] + res[s + 2] <= wall + 10, "capped process %d: %u ticks running + %u throttled exceeds the %u elapsed", k, res[s + 0], res[s + 2], wall);
    }
    CHECK(res[R_C + 0] >= 3 * res[R_A + 0], "the unlimited process (%u) is not far ahead of the capped one (%u)", res[R_C + 0], res[R_A + 0]);
    CHECK(res[R_C + 1] == 0 && res[R_C + 2] == 0, "an unlimited process must never be throttled (%u windows)", res[R_C + 1]);
    if (failures == f0) printf("[rlimtest] ok: CPU share - processes capped at 20%% got about a fifth of a CPU and were throttled window after window; an unlimited one was untouched\n");
}

/* ---- group 5: process count -------------------------------------------------------------------- */

static void gated_child(unsigned int slot) {
    int start = now_s();
    while (res[R_GATE + slot] == 0 && elapsed_s(start) < 30) sys_yield();
    sys_exit(0);
}

/* Members of the bomb hold their slots until the parent opens this gate, so the
 * group is genuinely FULL while the bomb keeps trying - not merely full for the
 * few microseconds a short-lived child would linger on an idle machine. */
enum { R_BOMB_GATE = R_GATE + 7 };

static void hold_until_released(void) {
    int t = now_s();
    while (res[R_BOMB_GATE] == 0 && elapsed_s(t) < 20) sys_yield();
}

/* A deliberately finite fork bomb: every process forks six times, and its
 * children do the same, three levels deep. */
static void bomb(int depth) {
    for (int i = 0; i < 6; i++) {
        int pid = sys_fork();
        if (pid == 0) {
            if (depth < 2) bomb(depth + 1);
            hold_until_released();
            sys_exit(0);
        }
        __sync_fetch_and_add(&res[R_COUNT], 1);          /* attempts */
        if (pid < 0) __sync_fetch_and_add(&res[R_COUNT + 1], 1); /* refusals */
    }
}

/* The cap must hold on the second CPU too, where there is no timer: the BSP's
 * tick sends an IPI only while a limited process runs there. Three processes
 * capped at 10% run together, so with two CPUs at least one of them spends
 * time on the second CPU, and each must still get exactly its tenth of a CPU.
 * The test requires that the second CPU's tick enforced at least one of them
 * (ap_ticks, the kernel's own evidence), so a run in which nothing was ever
 * enforced there cannot pass. */
static void test_cap_on_the_second_cpu(void) {
    int f0 = failures;
    memset((void*)res, 0, 64 * sizeof(unsigned int));
    int pid[3];
    unsigned int slot[3] = { R_A, R_B, R_C };
    for (int i = 0; i < 3; i++) {
        pid[i] = sys_fork();
        if (pid[i] == 0) spinner(10, slot[i]);
        CHECK(pid[i] > 0, "fork failed");
    }
    int code[3] = { -1, -1, -1 };
    for (int i = 0; i < 3; i++) {
        CHECK(wait_exit(pid[i], 15, &code[i]) && code[i] == 0, "spinner %d did not finish (code %d)", i, code[i]);
    }
    unsigned int wall = res[R_A + 3] ? res[R_A + 3] : 400, parks = 0, ap_ticks = 0;
    unsigned int want = wall / 10;
    for (int i = 0; i < 3; i++) {
        unsigned int cpu = res[slot[i] + 0];
        parks += res[slot[i] + 5];
        ap_ticks += res[slot[i] + 4];
        CHECK(cpu >= want * 55 / 100 && cpu <= want * 150 / 100,
              "spinner %d capped at 10%% used %u CPU ticks of %u elapsed; expected about %u (accepting %u to %u)", i, cpu, wall, want, want * 55 / 100, want * 150 / 100);
    }
    printf("[rlimtest] note: three 10%% spinners used %u, %u and %u CPU ticks; the second CPU's tick enforced %u times, a throttled process waited there %u times\n",
           res[R_A + 0], res[R_B + 0], res[R_C + 0], ap_ticks, parks);
    CHECK(ap_ticks > 0, "no spinner was ever enforced from the second CPU: this test did not exercise it");
    /* `parks` is reported, NOT asserted. Whether the throttled process on the
     * second CPU ever finds itself with nothing else to run depends on where
     * the round-robin scheduler happens to have put everyone (it cycles the
     * parent through the idle task, which makes the parent briefly eligible
     * for that CPU), and an assertion that depends on that is a flaky test.
     * So the WAIT-in-the-interrupt branch of rlimit_ipi_handler() has no
     * assertion behind it; the fault-injection run that removes it survives
     * (see PROGRESS.md). */
    if (failures == f0) printf("[rlimtest] ok: CPU cap on the second CPU - three 10%% spinners stayed near a tenth of a CPU, enforced from both CPUs\n");
}

static void test_process_count(void) {
    int f0 = failures;
    memset((void*)res, 0, 128 * sizeof(unsigned int));
    nova_rlimit_t u;
    EXPECT(rl_set(0, PROC, 0, 0, 0, 4), 0);
    int code0 = 0;
    int pids[4];
    for (int i = 0; i < 4; i++) {
        pids[i] = sys_fork();
        if (pids[i] == 0) gated_child((unsigned)i);
        CHECK(pids[i] > 0, "member %d was refused below the cap (%d)", i, pids[i]);
    }
    EXPECT(rl_get(0, &u), 0);
    CHECK(u.procs == 4, "the group has %u live members, expected 4", u.procs);
    int refused = sys_fork();
    CHECK(refused < 0, "a 5th member was allowed (pid %d)", refused);
    if (refused == 0) sys_exit(0);
    /* (Spawn - sys_exec - is guarded by the same check as fork, at the top of
     * process_exec_internal(). It is NOT exercised here: a plain-exec'd program
     * has no spawn capability, so it would be refused for that reason and the
     * assertion would pass whether or not the limit worked.) */
    EXPECT(rl_get(0, &u), 0);
    CHECK(u.procs == 4, "a refused fork must not leave anything behind (%u members)", u.procs);
    /* A refusal must cost NOTHING. Refusing after a process-table slot has been
     * claimed would leak that slot and burn a pid for every refusal, and a fork
     * bomb would drain the table just by being refused. Ten more refusals, then
     * a member leaves and a new one is forked: its pid must follow the last one
     * (a few may be taken by system processes starting meanwhile; ten leaked
     * slots would show as a gap of ten or more). */
    for (int i = 0; i < 10; i++) {
        int r = sys_fork();
        if (r == 0) sys_exit(0);
        CHECK(r < 0, "refusal %d: a 5th member was allowed", i);
    }
    res[R_GATE + 0] = 1;
    CHECK(wait_exit(pids[0], 10, &code0), "member 0 did not exit");
    res[R_GATE + 0] = 0;
    pids[0] = sys_fork();
    if (pids[0] == 0) gated_child(0);
    CHECK(pids[0] > 0, "the group did not recover after a member left");
    CHECK(pids[0] - pids[3] <= 6, "10 refused forks burned %d pids: each refusal leaked a process slot", pids[0] - pids[3]);

    /* it recovers the moment one exits, and does not drift over many cycles */
    int code;
    int drift = 0, refork_ok = 0, still_capped = 0;
    (void)code0;
    for (int cycle = 0; cycle < 20; cycle++) {
        int victim = cycle % 4;
        res[R_GATE + victim] = 1;
        if (!wait_exit(pids[victim], 10, &code)) { drift++; break; }
        res[R_GATE + victim] = 0;
        pids[victim] = sys_fork();
        if (pids[victim] == 0) gated_child((unsigned)victim);
        if (pids[victim] > 0) refork_ok++;
        int extra = sys_fork();
        if (extra < 0) still_capped++;
        if (extra == 0) sys_exit(0);
        if (rl_get(0, &u) != 0 || u.procs != 4) drift++;
    }
    CHECK(refork_ok == 20, "only %d of 20 exit-and-refork cycles succeeded: the count drifted upward", refork_ok);
    CHECK(still_capped == 20, "the cap held in only %d of 20 cycles", still_capped);
    CHECK(drift == 0, "the member count was wrong in %d cycles", drift);

    /* a member that forks: it counts against the SAME group, and is refused at the cap */
    res[R_GATE + 0] = 1;
    CHECK(wait_exit(pids[0], 10, &code), "member 0 did not exit");
    res[R_GATE + 0] = 0;
    int mid = sys_fork();
    if (mid == 0) {
        failures = 0;
        int g1 = sys_fork();   /* the 4th member: 3 siblings + this one + this child = 5 > 4? no: 3 + mid = 4 already */
        if (g1 == 0) sys_exit(0);
        res[R_COUNT + 8] = (g1 < 0) ? 1 : 0; /* 1 = refused, as it must be: the group is full */
        sys_exit(0);
    }
    CHECK(mid > 0, "the replacement member was refused");
    CHECK(wait_exit(mid, 10, &code) && code == 0, "the grandchild-forker failed");
    CHECK(res[R_COUNT + 8] == 1, "a member's own child was allowed past the group's cap");

    /* release everyone, then run a genuine bomb against the same cap */
    for (int i = 0; i < 4; i++) res[R_GATE + i] = 1;
    for (int i = 0; i < 4; i++) { if (pids[i] > 0) wait_exit(pids[i], 10, &code); }
    EXPECT(rl_get(0, &u), 0);
    CHECK(u.procs == 0, "after everyone left the group has %u members", u.procs);

    res[R_BOMB_GATE] = 0;
    int root = sys_fork();
    if (root == 0) {
        bomb(0);
        hold_until_released();
        sys_exit(0);
    }
    CHECK(root > 0, "the bomb's first process was refused");
    /* watch the group while the bomb fights the cap; open the gate only once the
     * group has been SEEN at its cap and the bomb has had time to be refused a
     * good many times, then watch it drain */
    unsigned int max_seen = 0, samples = 0;
    int released = 0;
    int start = now_s();
    while (elapsed_s(start) < 14) {
        if (rl_get(0, &u) == 0) {
            samples++;
            if (u.procs > max_seen) max_seen = u.procs;
            if (!released && (max_seen == 4 && res[R_COUNT + 1] >= 12 && samples > 30)) {
                res[R_BOMB_GATE] = 1;
                released = 1;
            }
            if (released && u.procs == 0) break;
        }
        if (!released && elapsed_s(start) >= 8) { res[R_BOMB_GATE] = 1; released = 1; }
        sys_yield();
    }
    res[R_BOMB_GATE] = 1;
    wait_exit(root, 10, &code);
    printf("[rlimtest] note: a fork bomb made %u fork attempts, %u were refused; the group never had more than %u of its 4 members alive (%u samples)\n",
           res[R_COUNT], res[R_COUNT + 1], max_seen, samples);
    CHECK(max_seen <= 4, "the bomb got %u processes alive, the cap is 4", max_seen);
    CHECK(max_seen == 4, "the bomb never reached the cap (%u): the test did not exercise it", max_seen);
    CHECK(res[R_COUNT + 1] > 10, "only %u of the bomb's forks were refused", res[R_COUNT + 1]);
    EXPECT(rl_get(0, &u), 0);
    CHECK(u.procs == 0, "the bomb left %u members behind", u.procs);

    /* lifting the cap (root may) and the group counts nothing */
    EXPECT(rl_set(0, PROC, 0, 0, 0, 0), 0);
    if (failures == f0) printf("[rlimtest] ok: process count - a group capped at 4 refuses a 5th however it is made, recovers when a member exits with no drift over 20 cycles, and a fork bomb never exceeded it\n");
}

/* ---- group 6: confinement and hygiene --------------------------------------------------------------- */

static void confined_child(void) {
    failures = 0;
    nova_rlimit_t u;
    EXPECT(rl_get(0, &u), 0);
    CHECK(u.quota_root != 0, "a member of a capped group must know its quota root");
    CHECK(sys_login("persisted", "persisted-pw") == 0, "login");
    /* a confined non-root process may not start a fresh allowance for itself */
    EXPECT(rl_set(0, PROC, 0, 0, 0, 100), -EPERM);
    sys_exit(failures);
}

static void hygiene_child(void) {
    failures = 0;
    nova_rlimit_t u;
    EXPECT(rl_get(0, &u), 0);
    CHECK(u.max_vm_pages == 0 && u.cpu_percent == 0 && u.cpu_time_ticks == 0 && u.max_procs == 0, "a recycled slot kept limits: %u %u %u %u",
          u.max_vm_pages, u.cpu_percent, u.cpu_time_ticks, u.max_procs);
    CHECK(u.throttled == 0 && u.throttle_events == 0 && u.throttled_ticks == 0 && u.mem_denied == 0, "a recycled slot kept a throttle or usage");
    CHECK(u.quota_root == 0, "a recycled slot kept a quota root: %d", u.quota_root);
    sys_exit(failures);
}

static void test_confinement_and_hygiene(void) {
    int f0 = failures;
    int code;
    EXPECT(rl_set(0, PROC, 0, 0, 0, 6), 0);
    int pid = sys_fork();
    if (pid == 0) confined_child();
    CHECK(pid > 0 && wait_exit(pid, 10, &code) && code == 0, "the confined child failed (code %d)", code);
    EXPECT(rl_set(0, PROC, 0, 0, 0, 0), 0);

    /* thirty fresh processes reuse the slots the dead hogs and bombs left. None may
     * inherit anything from a previous occupant. */
    int bad = 0;
    for (int i = 0; i < 30; i++) {
        int p = sys_fork();
        if (p == 0) hygiene_child();
        if (p < 0) { bad++; continue; }
        if (!wait_exit(p, 10, &code) || code != 0) bad++;
    }
    CHECK(bad == 0, "%d of 30 processes in recycled slots started with leftovers (or could not run)", bad);
    if (failures == f0) printf("[rlimtest] ok: confinement and hygiene - a confined user cannot start its own allowance, and 30 processes in recycled slots started with no limits, throttle or usage\n");
}

int main(void) {
    EXPECT(nova_shm_create(&mbr, 4096), 0);
    res = (volatile unsigned int*)mbr.addr;

    test_api_and_privilege();
    test_memory();
    test_cpu_time();
    test_cpu_share();
    test_cap_on_the_second_cpu();
    test_process_count();
    test_confinement_and_hygiene();

    nova_shm_detach(&mbr);
    sys_shm_destroy(mbr.handle);

    if (failures == 0) {
        printf("[rlimtest] PASS: per-process resource limits hold (%d checks)\n", checks);
        return 0;
    }
    printf("[rlimtest] FAIL: %d of %d checks failed\n", failures, checks);
    return 1;
}
