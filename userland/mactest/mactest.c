/*
 * mactest.c - Phase 87: the in-OS conformance test for mandatory access
 * control (SYS_MAC_INFO / SYS_MAC_CTL and the profiles themselves). A ring-3
 * program against the real kernel, in the role of the UNCONFINED ROOT - the
 * policy administrator - who starts a confined program (userland/macjail) and
 * checks what the kernel did to it. kernel/task/exec_trust_demo.c runs it at
 * boot and tools/python/test_runner.py checks its "[mactest] ok:" lines.
 *
 * It is the in-OS counterpart of kernel/rust/mac.rs's host tests: those prove
 * the policy ENGINE (parsing, matching, stacks, the table) against a reference;
 * this proves the whole stack - the syscall gate, the argument checks on the
 * kernel's copy of each name, the profile being found and loaded when a
 * program starts, fork and exec carrying confinement down, the policy files
 * being protected, the freeze.
 *
 * The scenarios, each a way MAC could be wrong that "it returned 0" would miss:
 *
 *  - ADMIN AND POLICY FILES: only an unconfined root may write or delete a
 *    *.MAC file. A non-root user with every capability may not; neither may a
 *    confined root, however permissive its profile.
 *  - THE JAIL: a ROOT process with its capability lists wide open is bound by
 *    a tight profile. macjail sweeps every syscall number its profile does not
 *    list and every one must be refused AND counted against that number; it
 *    reads, writes, deletes, sends, binds, connects and starts programs on and
 *    off its list; it forks. All of it is checked by the kernel's own denial
 *    counters, not by return values alone.
 *  - NO ESCAPE BY EXEC: the jail starts a program whose OWN profile allows
 *    everything. The child is bound by both: the permissive profile widens
 *    nothing.
 *  - TWO LAYERS: a permissive profile with NO capabilities still cannot open
 *    a file (MAC never grants what the capability model refuses).
 *  - COMPLAIN MODE: what a profile does not list is logged and allowed, and
 *    counted as a complaint, never as a denial.
 *  - FAIL CLOSED: a program whose profile does not parse is not started.
 *  - THE STACK LIMIT: a chain of programs each under one more profile; the one
 *    that would be the fifth is refused.
 *  - FREEZE: a profile written before the freeze loads; after it nothing new
 *    can load and no policy file can be touched, even by root - and profiles
 *    already loaded still work. (Last, because it cannot be undone until reboot.)
 */
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
            printf("[mactest] FAIL: %s (line %d) ", #cond, __LINE__); \
            printf(__VA_ARGS__); \
            printf("\n"); \
        } \
    } \
} while (0)

#define NOSPAWN (-1000)

static nova_mac_info_t info(void) {
    nova_mac_info_t i;
    memset(&i, 0, sizeof i);
    (void)sys_mac_info(&i);
    return i;
}

/* Starts `path` (trusted = it inherits this process's capabilities, otherwise
 * it gets none) and returns its exit code, or NOSPAWN if the kernel refused to
 * start it. */
static int run(const char* path, const char* mode, int trusted) {
    char* argv[] = { (char*)path, (char*)mode };
    int pid = trusted ? sys_exec_trusted(path, argv, 2) : sys_exec(path, argv, 2);
    if (pid < 0) return NOSPAWN;
    return sys_wait(pid);
}

static int run_deep(void) {
    char* argv[] = { "MACD1.ELF", "deep", "1" };
    int pid = sys_exec_trusted("MACD1.ELF", argv, 3);
    if (pid < 0) return NOSPAWN;
    return sys_wait(pid);
}

static const char GOOD_PROFILE[] = "mode enforce\nallow syscall write yield\n";

/* ---- admin and policy files --------------------------------------------------------- */

static void nonroot_child(void) {
    failures = 0;
    CHECK(sys_login("persisted", "persisted-pw") == 0, "login");
    CHECK(sys_getuid() == 700, "uid %u", sys_getuid());
    nova_mac_info_t b = info();
    /* it has every capability (a fork copies them), so only MAC can refuse */
    CHECK(sys_write_file("MACNEW.MAC", GOOD_PROFILE, sizeof GOOD_PROFILE - 1) == -1, "a non-root user must not write a policy file");
    nova_mac_info_t a = info();
    CHECK(a.denied == b.denied + 1 && a.last_kind == NOVA_MAC_KIND_POLICY, "denied as a POLICY write (denials %u->%u, kind %u)", b.denied, a.denied, a.last_kind);
    CHECK(sys_delete_file("MACJAIL.MAC") == -1, "nor delete one");
    CHECK(sys_mac_ctl(NOVA_MAC_CTL_IS_ADMIN, 0) == 0, "a non-root user is not the MAC administrator");
    CHECK(sys_mac_ctl(NOVA_MAC_CTL_FREEZE, 0) == -1, "nor may it freeze the policy");
    CHECK(info().frozen == 0, "so the policy is not frozen");
    /* ... but an ordinary file is still its to write: policy files are special, not all files */
    CHECK(sys_write_file("NOTPOL.TXT", "x", 1) == 1, "an ordinary file is still writable (capability model)");
    CHECK(sys_delete_file("NOTPOL.TXT") == 1, "and deletable");
    sys_exit(failures);
}

static void test_admin_and_policy_files(void) {
    int f0 = failures;
    nova_mac_info_t i = info();
    CHECK(i.depth == 0, "the driver is the unconfined administrator (depth %u)", i.depth);
    CHECK(sys_getuid() == 0, "and root (uid %u)", sys_getuid());
    CHECK(sys_mac_ctl(NOVA_MAC_CTL_IS_ADMIN, 0) == 1, "an unconfined root is the MAC administrator");
    CHECK(i.frozen == 0, "the policy starts unfrozen");
    (void)sys_delete_file("JAILOUT.TXT"); /* a leftover from an earlier boot of this disk */
    CHECK(sys_write_file("MACNEW.MAC", GOOD_PROFILE, sizeof GOOD_PROFILE - 1) == 1, "the administrator may write a policy file");
    CHECK(sys_delete_file("MACNEW.MAC") == 1, "and delete it");
    CHECK(info().denied == 0, "the administrator is never denied (%u)", info().denied);
    int pid = sys_fork();
    if (pid == 0) nonroot_child();
    CHECK(pid > 0, "fork failed");
    int code = -1;
    if (pid > 0) code = sys_wait(pid);
    CHECK(code == 0, "the non-root child reported %d failed checks", code);
    if (failures == f0) printf("[mactest] ok: policy files - an unconfined root may write them; a non-root user with every capability may not, nor may it freeze the policy\n");
}

/* ---- the jail ---------------------------------------------------------------------------- */

static void test_jail(void) {
    int f0 = failures;
    nova_mac_info_t before = info();
    int code = run("MACJAIL.ELF", "probe", 1);
    CHECK(code != NOSPAWN, "the jail could not be started at all");
    CHECK(code == 0, "the jail reported %d failed checks (its own lines say which)", code);
    nova_mac_info_t after = info();
    /* the jail's own 72, plus what its child, its forked child and ITS forked child were refused */
    CHECK(after.total_denied >= before.total_denied + 78, "the kernel's system-wide denial count rose by only %u", after.total_denied - before.total_denied);
    CHECK(after.loaded >= before.loaded + 2, "the jail's and its child's profiles were loaded (%u -> %u)", before.loaded, after.loaded);
    CHECK(after.denied == 0, "the driver itself is unconfined and was never denied (%u)", after.denied);
    /* the one write it was allowed to make really happened, and the administrator can clean it up */
    CHECK(sys_delete_file("JAILOUT.TXT") == 1, "the jail's permitted write to JAILOUT.TXT did not happen");
    /* the files it was refused were never touched */
    CHECK(sys_delete_file("OTHER.TXT") == -1, "OTHER.TXT must not exist: the jail was refused it");
    if (failures == f0) printf("[mactest] ok: the jail - a root process with every capability was refused each syscall it did not list and every file, peer and program off its list - 72 refusals, each counted against the right thing\n");
}

static void test_no_escape_by_exec(void) {
    int f0 = failures;
    /* the permissive profile run ON ITS OWN allows what the jail forbade ... */
    int code = run("MACKID.ELF", "alone", 1);
    CHECK(code == 0, "the permissive profile alone reported %d failed checks", code);
    /* ... and the jail test above already ran it UNDER the jail and required it to be refused */
    if (failures == f0) printf("[mactest] ok: no escape by exec - the permissive profile alone allows SECRET.TXT, yet the same program started by the jail was bound by both profiles; a confined root is never the administrator\n");
}

static void test_two_layers(void) {
    int f0 = failures;
    int code = run("MACLITE.ELF", "lite", 0); /* NO capabilities */
    CHECK(code == 0, "the no-capability program reported %d failed checks", code);
    if (failures == f0) printf("[mactest] ok: two layers - a profile that allows everything granted nothing to a program whose capabilities refuse it, and MAC counted no denial\n");
}

static void test_complain_mode(void) {
    int f0 = failures;
    int code = run("MACCOMP.ELF", "comp", 1);
    CHECK(code == 0, "the complain-mode program reported %d failed checks", code);
    if (failures == f0) printf("[mactest] ok: complain mode - what the profile did not list was allowed, reported and counted as a complaint, never as a denial\n");
}

static void test_fail_closed_and_stack_limit(void) {
    int f0 = failures;
    nova_mac_info_t b = info();
    int code = run("MACBAD.ELF", "bad", 1);
    CHECK(code == NOSPAWN, "a program whose profile does not parse was STARTED (it returned %d)", code);
    CHECK(info().loaded == b.loaded, "an invalid profile must not be loaded");
    /* the same program under a profile that does parse starts fine: it was the profile */
    CHECK(run("MACKID.ELF", "alone", 1) == 0, "an ordinary program still starts");
    int deep = run_deep();
    CHECK(deep == 44, "the chain of programs ended with code %d; 44 means the fifth profile was refused and 55 means it was not", deep);
    if (failures == f0) printf("[mactest] ok: fails closed - a program with an unparseable profile never started; a chain of programs was stopped at the 4th profile, not silently weakened\n");
}

/* ---- the freeze (last: it cannot be undone until reboot) -------------------------------------- */

static void test_freeze(void) {
    int f0 = failures;
    static const char FRZ[] = "mode enforce\nallow syscall write yield\n";
    CHECK(sys_write_file("MACFRZ.MAC", FRZ, sizeof FRZ - 1) == 1, "the administrator writes a new profile before the freeze");
    nova_mac_info_t b = info();
    CHECK(run("MACFRZ.ELF", "frz", 1) == 0, "a program with a profile written at run time starts and runs");
    CHECK(info().loaded == b.loaded + 1, "and its profile was loaded (%u -> %u)", b.loaded, info().loaded);
    CHECK(sys_mac_ctl(NOVA_MAC_CTL_FREEZE, 0) == 0, "the administrator freezes the policy");
    CHECK(info().frozen == 1, "and it is frozen");
    /* frozen: not even root may touch a policy file ... */
    nova_mac_info_t d0 = info();
    CHECK(sys_write_file("MACNEW.MAC", GOOD_PROFILE, sizeof GOOD_PROFILE - 1) == -1, "after the freeze even root may not write a policy file");
    CHECK(sys_delete_file("MACFRZ.MAC") == -1, "nor delete one");
    CHECK(info().denied == d0.denied + 2 && info().last_kind == NOVA_MAC_KIND_POLICY, "both counted as POLICY denials");
    CHECK(sys_write_file("STILLOK.TXT", "x", 1) == 1, "an ordinary file is still writable");
    CHECK(sys_delete_file("STILLOK.TXT") == 1, "and deletable");
    /* ... a profile never loaded before the freeze cannot be loaded now ... */
    nova_mac_info_t l0 = info();
    CHECK(run("MACLATE.ELF", "frz", 1) == NOSPAWN, "a program whose profile was never loaded must not start once the policy is frozen");
    CHECK(info().loaded == l0.loaded, "nothing was loaded");
    /* ... and profiles already loaded keep working: the freeze stops change, not use */
    CHECK(run("MACFRZ.ELF", "frz", 1) == 0, "an already-loaded profile still works");
    CHECK(run("MACKID.ELF", "alone", 1) == 0, "so do the others");
    CHECK(run("MACJAIL.ELF", "probe", 1) == 0, "including the whole jail battery, rerun after the freeze");
    CHECK(sys_delete_file("JAILOUT.TXT") == 1, "(cleaning up after it)");
    CHECK(sys_mac_ctl(NOVA_MAC_CTL_FREEZE, 0) == 0, "freezing again is harmless");
    CHECK(info().frozen == 1, "there is no way to undo it");
    if (failures == f0) printf("[mactest] ok: freeze - policy files untouchable even by root, a never-loaded profile refused, already-loaded profiles (and the whole jail battery) still work\n");
}

static int now_s(void) {
    nova_rtc_time_t t;
    sys_rtc_read(&t);
    return t.hour * 3600 + t.minute * 60 + t.second;
}

int main(void) {
    int t0 = now_s(), t1, t2, t3, t4, t5, t6;
    test_admin_and_policy_files();
    t1 = now_s();
    test_jail();
    t2 = now_s();
    test_no_escape_by_exec();
    test_two_layers();
    test_complain_mode();
    t3 = now_s(); t4 = t3;
    test_fail_closed_and_stack_limit();
    t5 = now_s();
    test_freeze();
    t6 = now_s();
    (void)t4;
    printf("[mactest] note: seconds per group - admin %d, jail %d, exec/layers/complain %d, fail-closed/chain %d, freeze %d\n",
           t1 - t0, t2 - t1, t3 - t2, t5 - t3, t6 - t5);

    if (failures == 0) {
        printf("[mactest] PASS: mandatory access control holds (%d checks)\n", checks);
        return 0;
    }
    printf("[mactest] FAIL: %d of %d checks failed\n", failures, checks);
    return 1;
}
