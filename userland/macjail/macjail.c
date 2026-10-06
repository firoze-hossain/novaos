/*
 * macjail.c - Phase 87: the CONFINED program of the mandatory-access-control
 * conformance test. mactest.c (the driver, an unconfined root) starts it; this
 * is the thing the profile has to hold against, so it behaves like the worst
 * a buggy or taken-over program could: it runs as ROOT, with its capability
 * lists wide open (it is started with sys_exec_trusted from a process that
 * has can_open_any_file and can_spawn), and it then tries to do everything.
 *
 * ONE binary, installed on the disk under several names, because a profile is
 * bound to the program NAME (tools/build-disk-image.sh copies it):
 *
 *   MACJAIL   a tight profile, mode "probe"  - the main battery
 *   MACKID    a PERMISSIVE profile, modes "kid" (started BY the jail) and
 *             "alone" (started by the driver)
 *   MACCOMP   a complain-mode profile, mode "comp"
 *   MACLITE   a permissive profile but NO capabilities, mode "lite"
 *   MACBAD    a profile that does not parse: it must never run at all
 *   MACD1..5  a chain of programs each starting the next, mode "deep N"
 *   MACFRZ    a profile the driver writes at run time, mode "frz"
 *
 * Each check below asks the kernel, via SYS_MAC_INFO, what it denied and why,
 * so "the call returned -1" is never taken for "MAC refused it": a call can
 * fail for other reasons, and the point is to see the denial COUNTED against
 * the right kind of thing. A confined process always may call mac_info and
 * exit; it prints with write.
 *
 * Every failure prints "[macjail] FAIL: ..." (which also trips the test
 * runner's global no-fail scan); passing output deliberately avoids the words
 * that scan looks for.
 */
#include <novasys.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int checks, failures;

#define FAILMSG(...) do { \
    failures++; \
    if (failures <= 20) { \
        printf("[macjail] FAIL: "); \
        printf(__VA_ARGS__); \
        printf(" (line %d)\n", __LINE__); \
    } \
} while (0)

#define CHECK(cond, ...) do { checks++; if (!(cond)) FAILMSG(__VA_ARGS__); } while (0)

static int raw(unsigned n, unsigned a, unsigned b, unsigned c) {
    int r;
    __asm__ volatile ("int $0x80" : "=a"(r) : "a"(n), "b"(a), "c"(b), "d"(c) : "memory");
    return r;
}

static nova_mac_info_t info(void) {
    nova_mac_info_t i;
    memset(&i, 0, sizeof i);
    (void)sys_mac_info(&i);
    return i;
}

/* The call must return -1 AND the kernel must have counted exactly one denial
 * of this kind and argument against THIS process. */
#define DENIED(call, kind_, arg_) do { \
    nova_mac_info_t b_ = info(); \
    int r_ = (call); \
    nova_mac_info_t a_ = info(); \
    checks++; \
    if (!(r_ == -1 && a_.denied == b_.denied + 1 && a_.last_kind == (unsigned)(kind_) && a_.last_arg == (unsigned)(arg_))) \
        FAILMSG("%s returned %d; denials %u->%u, last kind %u arg %u (wanted kind %u arg %u)", \
                #call, r_, b_.denied, a_.denied, a_.last_kind, a_.last_arg, (unsigned)(kind_), (unsigned)(arg_)); \
} while (0)

/* MAC must NOT have objected: the denial count is unchanged. (The call may
 * still fail for its own reasons - no network, say - and that is not MAC's.) */
#define PERMITTED(var, call) \
    nova_mac_info_t var##_b = info(); \
    int var = (call); \
    do { \
        nova_mac_info_t var##_a = info(); \
        checks++; \
        if (var##_a.denied != var##_b.denied) \
            FAILMSG("%s was refused by MAC (denials %u->%u, kind %u arg %u)", #call, var##_b.denied, var##_a.denied, var##_a.last_kind, var##_a.last_arg); \
    } while (0)

static int is_in(const unsigned* set, int n, unsigned v) {
    for (int i = 0; i < n; i++) if (set[i] == v) return 1;
    return 0;
}

/* The syscalls MACJAIL.MAC lists (kept in step with tools/fixtures/mac/MACJAIL.MAC),
 * plus the two a profile never needs to list. */
static const unsigned jail_allowed[] = {
    1 /*write*/, 3 /*yield*/, 4 /*open*/, 5 /*read*/, 6 /*close*/, 11 /*sbrk*/, 12 /*fork*/,
    10 /*wait*/, 41 /*wait_nonblock*/, 30 /*getuid*/, 15 /*rtc_read*/, 18 /*write_file*/,
    19 /*delete_file*/, 9 /*exec*/, 38 /*exec_trusted*/, 42 /*exec_env*/, 43 /*exec_trusted_env*/, 44 /*socket_udp*/, 45 /*sendto*/,
    34 /*bind*/, 37 /*connect*/, 27 /*pipe*/, 2 /*exit*/, 71 /*mac_info*/,
};

#define KIND_SYSCALL NOVA_MAC_KIND_SYSCALL
#define KIND_FILE    NOVA_MAC_KIND_FILE
#define KIND_NET     NOVA_MAC_KIND_NET
#define KIND_EXEC    NOVA_MAC_KIND_EXEC
#define KIND_POLICY  NOVA_MAC_KIND_POLICY

static int name_is(const char* got, const char* want) {
    return strncmp(got, want, NOVA_MAC_NAME_LEN) == 0;
}

/* ---- mode "probe": the main battery, as the tight profile ------------------------- */

static int mode_probe(void) {
    nova_mac_info_t i0 = info();
    CHECK(i0.depth == 1, "the jail should be under exactly 1 profile, not %u", i0.depth);
    CHECK(name_is(i0.names[0], "MACJAIL"), "its profile is '%s'", i0.names[0]);
    CHECK(sys_getuid() == 0, "this test is meant to run as ROOT (uid %u): MAC applies to root too", sys_getuid());
    CHECK(i0.denied == 0 && i0.complained == 0, "nothing should have been denied yet (%u)", i0.denied);

    /* 1. what the profile lists works, and is not counted as a denial */
    PERMITTED(uid, (int)sys_getuid());
    nova_rtc_time_t t;
    PERMITTED(rtc, sys_rtc_read(&t));
    void* mem = (void*)0;
    {
        nova_mac_info_t b = info();
        mem = sys_sbrk(4096);
        nova_mac_info_t a = info();
        CHECK(mem != (void*)-1 && a.denied == b.denied, "sbrk, which is listed, must work");
    }
    sys_yield();
    (void)uid; (void)rtc;

    /* 2. THE SWEEP: every syscall number the profile does not list, tried by a
     * root process whose capabilities allow everything. Each must be refused
     * AND counted against that exact number. exit and mac_info are always
     * allowed; everything else not listed is not. */
    int swept = 0;
    for (unsigned n = 1; n <= 72; n++) {
        if (is_in(jail_allowed, (int)(sizeof jail_allowed / sizeof jail_allowed[0]), n)) continue;
        DENIED(raw(n, 0, 0, 0), KIND_SYSCALL, n);
        swept++;
    }
    CHECK(swept >= 45, "only %d syscalls were swept", swept);
    /* numbers the kernel has never heard of are denied too, not passed through */
    static const unsigned unknown[] = { 73, 100, 127, 128, 255, 4096, 0x7FFFFFFF };
    for (unsigned k = 0; k < sizeof unknown / sizeof unknown[0]; k++) {
        DENIED(raw(unknown[k], 0, 0, 0), KIND_SYSCALL, unknown[k]);
    }

    /* 3. files: capability open-any, so only the profile can say no */
    {
        PERMITTED(h1, sys_open("JAIL.TXT"));
        CHECK(h1 >= 0, "JAIL.TXT is listed and the capability allows it, yet open returned %d", h1);
        if (h1 >= 0) sys_close(h1);
        PERMITTED(h2, sys_open("jail.txt"));
        CHECK(h2 >= 0, "file patterns are case-insensitive (%d)", h2);
        if (h2 >= 0) sys_close(h2);
        PERMITTED(h3, sys_open("NOTES.LOG"));
        CHECK(h3 >= 0, "*.LOG is read-listed (%d)", h3);
        if (h3 >= 0) sys_close(h3);
        DENIED(sys_open("SECRET.TXT"), KIND_FILE, 1);
        DENIED(sys_open("NOTES.TXT"), KIND_FILE, 1);
        DENIED(sys_open("MACJAIL.MAC"), KIND_FILE, 1);
        DENIED(sys_open(""), KIND_FILE, 1);
        /* hostile arguments: refused cleanly, never a kernel fault. The kernel
         * copies the name once, validated, so none of these can be read twice or
         * dereferenced blind. (They are refused for being invalid, not by a
         * profile rule, so no denial is counted.) */
        {
            char longname[100];
            memset(longname, 'A', sizeof longname - 1);
            longname[sizeof longname - 1] = '\0';
            nova_mac_info_t b = info();
            CHECK(sys_open(longname) == -1, "an over-long name must be refused");
            CHECK(sys_open((const char*)0) == -1, "a NULL name must be refused");
            CHECK(sys_open((const char*)0xC0000000u) == -1, "a kernel-space name pointer must be refused");
            CHECK(sys_open((const char*)0xFFFFFFFFu) == -1, "a wild name pointer must be refused");
            CHECK(sys_write_file((const char*)0x10, "x", 1) == -1, "a bad write_file name must be refused");
            CHECK(sys_delete_file((const char*)0xFFFFFFF0u) == -1, "a bad delete_file name must be refused");
            CHECK(info().denied == b.denied, "invalid arguments are not policy denials");
        }
    }

    /* 4. writes and deletes: rw on JAILOUT.TXT only, and d on nothing */
    {
        PERMITTED(w, sys_write_file("JAILOUT.TXT", "x", 1));
        CHECK(w == 1, "writing the one file it may write returned %d", w);
        DENIED(sys_write_file("OTHER.TXT", "x", 1), KIND_FILE, 2);
        DENIED(sys_write_file("JAIL.TXT", "x", 1), KIND_FILE, 2);        /* r only */
        DENIED(sys_delete_file("JAILOUT.TXT"), KIND_FILE, 4);             /* rw, not d */
        DENIED(sys_delete_file("HELLO.TXT"), KIND_FILE, 4);
        /* its own policy file - and the others - must be untouchable */
        DENIED(sys_write_file("MACJAIL.MAC", "allow syscall *\n", 16), KIND_FILE, 2);
        DENIED(sys_delete_file("MACJAIL.MAC"), KIND_FILE, 4);
        DENIED(sys_write_file("MACKID.MAC", "allow syscall *\n", 16), KIND_FILE, 2);
    }

    /* 5. the network, on a UDP socket (so nothing here waits on a peer) */
    {
        PERMITTED(s, sys_socket_udp());
        CHECK(s >= 0, "a UDP socket should open (%d)", s);
        nova_udp_addr_t ok = { 0x0A000202, 9 }, wrongport = { 0x0A000202, 10 }, wronghost = { 0x01020304, 9 };
        { PERMITTED(r, sys_sendto(s, &ok, "x", 1)); (void)r; }
        DENIED(sys_sendto(s, &wrongport, "x", 1), KIND_NET, 10);
        DENIED(sys_sendto(s, &wronghost, "x", 1), KIND_NET, 9);
        { PERMITTED(r, sys_bind(s, 7000)); (void)r; }
        DENIED(sys_bind(s, 7001), KIND_NET, 7001);
        DENIED(sys_connect(s, 0x0A000202, 80), KIND_NET, 80);             /* only send/bind are listed */
    }

    /* 6. starting programs: only MACKID.ELF is listed */
    {
        DENIED(sys_exec("HELLO.ELF", (char**)0, 0), KIND_EXEC, 9);
        DENIED(sys_exec_trusted("SHELL.ELF", (char**)0, 0), KIND_EXEC, 9);
        char* kid_argv[] = { "MACKID.ELF", "kid" };
        PERMITTED(kp, sys_exec_trusted("MACKID.ELF", kid_argv, 2));
        CHECK(kp > 0, "the listed program should start (%d)", kp);
        if (kp > 0) {
            int code = sys_wait(kp);
            CHECK(code == 0, "the child it started, now under BOTH profiles, reported %d failed checks", code);
        }
    }

    /* 7. a forked child is the same domain */
    {
        int pid = sys_fork();
        if (pid == 0) {
            failures = 0;
            nova_mac_info_t c = info();
            CHECK(c.depth == 1 && name_is(c.names[0], "MACJAIL"), "a forked child must be under the parent's profile (depth %u '%s')", c.depth, c.names[0]);
            CHECK(c.denied == 0, "its own denial count starts at zero (%u)", c.denied);
            DENIED(sys_beep(), KIND_SYSCALL, 17);
            DENIED(sys_open("SECRET.TXT"), KIND_FILE, 1);
            sys_exit(failures);
        }
        CHECK(pid > 0, "fork should work: it is listed (%d)", pid);
        if (pid > 0) {
            int code = sys_wait(pid);
            CHECK(code == 0, "the forked child reported %d failed checks", code);
        }
    }

    /* 8. the whole thing was counted */
    nova_mac_info_t fin = info();
    /* 48 swept syscalls + 7 unknown numbers + 4 + 7 file refusals + 4 network + 2 exec = 72 */
    CHECK(fin.denied == 72, "the jail made exactly 72 refused attempts and the kernel counted %u", fin.denied);
    CHECK(fin.total_denied >= fin.denied, "the system-wide count (%u) cannot be below this process's (%u)", fin.total_denied, fin.denied);
    CHECK(fin.complained == 0, "a process in enforce mode complains about nothing (%u)", fin.complained);
    printf("[macjail] note: the jail made %u denied attempts (%d syscalls swept), all counted\n", fin.denied, swept);
    return failures;
}

/* ---- mode "kid": started BY the jail, so under MACJAIL then MACKID ---------------- */

static int mode_kid(void) {
    nova_mac_info_t i = info();
    CHECK(i.depth == 2, "a child of the jail must be under 2 profiles, not %u", i.depth);
    CHECK(name_is(i.names[0], "MACJAIL") && name_is(i.names[1], "MACKID"), "stack is '%s','%s'", i.names[0], i.names[1]);
    /* MACKID.MAC alone would allow all of this. The jail beneath it does not. */
    DENIED(sys_beep(), KIND_SYSCALL, 17);
    DENIED(sys_open("SECRET.TXT"), KIND_FILE, 1);
    DENIED(sys_write_file("MACKID.MAC", "allow syscall *\n", 16), KIND_FILE, 2);
    DENIED(sys_exec("HELLO.ELF", (char**)0, 0), KIND_EXEC, 9);
    PERMITTED(h, sys_open("JAIL.TXT"));
    CHECK(h >= 0, "what BOTH profiles allow must work (%d)", h);
    if (h >= 0) sys_close(h);
    /* its own fork child is under both too */
    int pid = sys_fork();
    if (pid == 0) {
        failures = 0;
        nova_mac_info_t c = info();
        CHECK(c.depth == 2, "a forked grandchild must be under both profiles (%u)", c.depth);
        DENIED(sys_beep(), KIND_SYSCALL, 17);
        sys_exit(failures);
    }
    if (pid > 0) {
        int code = sys_wait(pid);
        CHECK(code == 0, "the grandchild reported %d failed checks", code);
    } else {
        FAILMSG("the kid could not fork");
    }
    return failures;
}

/* ---- mode "alone": the permissive profile by itself, started by the driver --------- */

static int mode_alone(void) {
    nova_mac_info_t i = info();
    CHECK(i.depth == 1 && name_is(i.names[0], "MACKID"), "alone: depth %u '%s'", i.depth, i.names[0]);
    CHECK(sys_getuid() == 0, "alone: uid %u", sys_getuid());
    /* here the profile allows everything, so what was refused above was the jail's doing */
    PERMITTED(h, sys_open("SECRET.TXT"));
    CHECK(h >= 0, "MACKID.MAC alone allows SECRET.TXT (%d)", h);
    if (h >= 0) sys_close(h);
    /* ... but a CONFINED root is never the policy administrator, however permissive its profile */
    CHECK(sys_mac_ctl(NOVA_MAC_CTL_IS_ADMIN, 0) == 0, "a confined root is not the MAC administrator");
    unsigned frozen_before = info().frozen; /* this also runs AFTER the freeze, so compare, do not assume 0 */
    DENIED(sys_write_file("MACPOL.MAC", "allow syscall *\n", 16), KIND_POLICY, 0);
    DENIED(sys_delete_file("MACJAIL.MAC"), KIND_POLICY, 0);
    DENIED(sys_mac_ctl(NOVA_MAC_CTL_FREEZE, 0), KIND_POLICY, NOVA_MAC_CTL_FREEZE);
    CHECK(info().frozen == frozen_before, "a confined root must not have changed the freeze state");
    return failures;
}

/* ---- mode "comp": complain mode - logged, allowed, never denied --------------------- */

static int mode_comp(void) {
    nova_mac_info_t i = info();
    CHECK(i.depth == 1 && name_is(i.names[0], "MACCOMP"), "comp: depth %u '%s'", i.depth, i.names[0]);
    unsigned c0 = i.complained;
    int r = sys_beep(); /* not listed: a strict profile would refuse it */
    nova_mac_info_t a = info();
    CHECK(r != -1, "complain mode must ALLOW the call (it returned %d)", r);
    CHECK(a.complained == c0 + 1, "and count it as a complaint (%u -> %u)", c0, a.complained);
    CHECK(a.denied == 0, "a complain-mode profile never denies (%u)", a.denied);
    int h = sys_open("SECRET.TXT"); /* no file rule at all */
    nova_mac_info_t b = info();
    CHECK(h >= 0, "complain mode lets the open through (%d)", h);
    if (h >= 0) sys_close(h);
    /* TWO complaints: the open SYSCALL is not listed, and no file rule matches */
    CHECK(b.complained == a.complained + 2 && b.denied == 0, "the open counts twice - syscall and file (%u -> %u, denied %u)", a.complained, b.complained, b.denied);
    CHECK(b.last_kind == 0, "nothing was DENIED, so there is no last denial (kind %u)", b.last_kind);
    printf("[macjail] note: complain mode let %u operations through and reported them\n", b.complained);
    return failures;
}

/* ---- mode "lite": permissive profile, NO capabilities ------------------------------- */

static int mode_lite(void) {
    nova_mac_info_t i = info();
    CHECK(i.depth == 1 && name_is(i.names[0], "MACLITE"), "lite: depth %u '%s'", i.depth, i.names[0]);
    /* The profile allows everything. The capability model does not. The two
     * layers are independent and BOTH must allow: MAC never grants. */
    int h = sys_open("HELLO.TXT");
    CHECK(h == -1, "with no capabilities the open must fail even though the profile allows it (%d)", h);
    CHECK(sys_write_file("X.TXT", "x", 1) == -1, "and so must a write");
    nova_mac_info_t a = info();
    CHECK(a.denied == 0, "the refusals were the CAPABILITY model's, not MAC's (MAC denied %u)", a.denied);
    return failures;
}

/* ---- mode "deep N": a chain of programs, each under one more profile ------------------- */

static int mode_deep(int level) {
    nova_mac_info_t i = info();
    if (i.depth != (unsigned)level) {
        printf("[macjail] FAIL: level %d is under %u profiles\n", level, i.depth);
        return 66;
    }
    if (level >= 5) return 55; /* must never be reached: the stack holds 4 */
    char name[16];
    strcpy(name, "MACD0.ELF");
    name[4] = (char)('0' + level + 1);
    char lv[4] = { (char)('0' + level + 1), 0, 0, 0 };
    char* a[] = { name, "deep", lv };
    int pid = sys_exec_trusted(name, a, 3);
    if (pid < 0) {
        /* the only level allowed to be refused is the one that would be the 5th profile */
        return level == 4 ? 44 : 66;
    }
    return sys_wait(pid);
}

int main(int argc, char** argv, char** envp) {
    (void)envp;
    const char* mode = argc > 1 ? argv[1] : "probe";
    int rc;
    if (strcmp(mode, "probe") == 0) rc = mode_probe();
    else if (strcmp(mode, "kid") == 0) rc = mode_kid();
    else if (strcmp(mode, "alone") == 0) rc = mode_alone();
    else if (strcmp(mode, "comp") == 0) rc = mode_comp();
    else if (strcmp(mode, "lite") == 0) rc = mode_lite();
    else if (strcmp(mode, "deep") == 0) rc = mode_deep(argc > 2 ? atoi(argv[2]) : 1);
    else if (strcmp(mode, "bad") == 0) {
        printf("[macjail] FAIL: a program whose profile does not parse was STARTED\n");
        rc = 99;
    } else if (strcmp(mode, "frz") == 0) rc = 0;
    else {
        printf("[macjail] FAIL: unknown mode '%s'\n", mode);
        rc = 98;
    }
    if (strcmp(mode, "probe") == 0 || strcmp(mode, "kid") == 0 || strcmp(mode, "alone") == 0 ||
        strcmp(mode, "comp") == 0 || strcmp(mode, "lite") == 0) {
        printf("[macjail] %s: %s (%d checks)\n", rc == 0 ? "ok" : "FAIL", mode, checks);
    }
    return rc;
}
