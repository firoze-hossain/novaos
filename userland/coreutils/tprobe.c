/*
 * tprobe.c - a tiny, deliberately minimal ring-3 program that exists
 * for exactly one reason: to give kernel/task/exec_trust_demo.c's own
 * SYS_EXEC_TRUSTED self-test a real, on-disk ELF to exec, so that
 * self-test exercises the genuine SYS_EXEC/SYS_EXEC_TRUSTED path (a
 * real `int 0x80`, a real ELF load, a real new process) rather than
 * something synthetic. Not a coreutil, not documented in any shell's
 * `help` text, and not something a person would ever run directly -
 * see PROGRESS.md's own Phase 59 entry for the full reasoning on why
 * this exists as a separate, tiny, permanent fixture rather than
 * folding this check into an existing program.
 *
 * Tries, in order: SYS_WRITE_FILE a new file, SYS_OPEN + SYS_READ it
 * back and check the content matches, then SYS_DELETE_FILE it and
 * confirm SYS_OPEN on the same name now fails. All four succeeding
 * needs can_open_any_file (see process.h) - exactly the capability
 * SYS_EXEC_TRUSTED delegates and plain SYS_EXEC never does. Returns 0
 * only if every step succeeded, 1 otherwise - the caller (kernel/task/
 * exec_trust_demo.c) checks this exit code via SYS_WAIT, not this
 * program's own output.
 */
#include <novasys.h>
#include <string.h>

int main(int argc, char** argv, char** envp) {
    (void)argc;
    (void)argv;
    (void)envp;

    const char* filename = "TPROBE.TXT";
    const char* content = "exec-trusted-probe";
    unsigned int len = (unsigned int)strlen(content);

    /* Every "denied"/"no" below is a real, deliberately-reachable
     * outcome (the whole point of the plain-SYS_EXEC half of this
     * self-test is to prove this probe gets denied), not an accident -
     * spelled in lowercase specifically so it can never spell the
     * uppercase substring "FAIL" and spuriously trip tools/python/
     * test_runner.py's own negative no_panic_fault_or_fail assertion,
     * the same case-sensitivity reasoning PROGRESS.md's own Phase 58
     * entry already applied to the pre-existing "TCP HTTP: ... failed"
     * WARN line. */
    int wr = sys_write_file(filename, content, len);
    sys_write("[tprobe] write=");
    sys_write(wr >= 0 ? "ok " : "denied ");
    if (wr < 0) {
        return 1; /* couldn't even write - no can_open_any_file */
    }

    int fd = sys_open(filename);
    sys_write("open1=");
    sys_write(fd >= 0 ? "ok " : "denied ");
    if (fd < 0) {
        return 1;
    }
    char buf[32];
    int n = sys_read(fd, buf, sizeof(buf) - 1);
    sys_close(fd);
    bool content_ok = (n == (int)len) && (memcmp(buf, content, len) == 0);
    sys_write("readback=");
    sys_write(content_ok ? "ok " : "no-match ");
    if (!content_ok) {
        return 1;
    }

    int del = sys_delete_file(filename);
    sys_write("delete=");
    sys_write(del >= 0 ? "ok " : "denied ");
    if (del < 0) {
        return 1;
    }

    /* Confirm it's really gone. Note: SYS_OPEN on this kernel is
     * deliberately lazy - it just claims a handle-table slot and
     * records the filename, without checking the file actually exists
     * on disk (that check only happens on the first SYS_READ against
     * the handle, see kernel/arch/x86/cpu/syscall.c's handle_read()).
     * So a bare "did SYS_OPEN return >= 0" check here would always
     * pass regardless of whether the file is really gone - the real
     * test is whether a read against it comes back empty. */
    int fd2 = sys_open(filename);
    bool really_gone = true;
    if (fd2 >= 0) {
        char tmp;
        int n2 = sys_read(fd2, &tmp, 1);
        sys_close(fd2);
        really_gone = (n2 < 0);
    }
    sys_write("confirm_gone=");
    sys_write(really_gone ? "ok\n" : "no-still-readable\n");
    if (!really_gone) {
        return 1;
    }

    return 0;
}
