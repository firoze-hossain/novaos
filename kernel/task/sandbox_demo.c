/*
 * sandbox_demo.c - proves Phase 11 and Phase 14's capability
 * enforcement is real
 *
 * This task is created (see kernel_main()) with a capability list
 * containing only "HELLO.TXT" for files and only the network gateway
 * (10.0.2.2) for network destinations. It deliberately tries both an
 * allowed and a disallowed case for each resource type and reports
 * PASS/FAIL for every one - a falsifiable test, not a claim, the same
 * approach Phase 5 used to prove address-space isolation actually
 * held.
 */
#include "sandbox_demo.h"
#include "../arch/x86/cpu/syscall.h"

static inline void sys_write(const char* str) {
    __asm__ volatile ("int $0x80" : : "a"(SYS_WRITE), "b"(str) : "memory", "cc");
}

static inline int sys_open(const char* filename) {
    int result = SYS_OPEN;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(filename)
                       : "memory", "cc");
    return result;
}

static inline int sys_read(int handle, void* buf, int max_len) {
    int result = SYS_READ;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(handle), "c"(buf), "d"(max_len)
                       : "memory", "cc");
    return result;
}

static inline void sys_close(int handle) {
    __asm__ volatile ("int $0x80"
                       :
                       : "a"(SYS_CLOSE), "b"(handle)
                       : "memory", "cc");
}

static inline int sys_net_send(unsigned int dest_ip, unsigned short dest_port,
                                const char* message) {
    int result = SYS_NET_SEND;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(dest_ip), "c"(dest_port), "d"(message)
                       : "memory", "cc");
    return result;
}

static inline int sys_spawn(void) {
    int result = SYS_SPAWN;
    __asm__ volatile ("int $0x80" : "+a"(result) : : "memory", "cc");
    return result;
}

static inline int sys_exec(const char* path, const char** argv, int argc) {
    int result = SYS_EXEC;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(path), "c"(argv), "d"(argc)
                       : "memory", "cc");
    return result;
}

static inline int sys_wait(int pid) {
    int result = SYS_WAIT;
    __asm__ volatile ("int $0x80" : "+a"(result) : "b"(pid) : "memory", "cc");
    return result;
}

static inline int sys_fork(void) {
    int result = SYS_FORK;
    __asm__ volatile ("int $0x80" : "+a"(result) : : "memory", "cc");
    return result;
}

static inline int sys_pipe(int out_handles[2]) {
    int result = SYS_PIPE;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(out_handles)
                       : "memory", "cc");
    return result;
}

static inline int sys_write_handle(int handle, const void* buf, int len) {
    int result = SYS_WRITE_HANDLE;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(handle), "c"(buf), "d"(len)
                       : "memory", "cc");
    return result;
}

static inline int sys_login(const char* username, const char* password) {
    int result = SYS_LOGIN;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(username), "c"(password)
                       : "memory", "cc");
    return result;
}

static inline unsigned int sys_getuid(void) {
    unsigned int result = SYS_GETUID;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       :
                       : "memory", "cc");
    return result;
}

static inline int sys_sudo(const char* password) {
    int result = SYS_SUDO;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(password)
                       : "memory", "cc");
    return result;
}

static inline void sys_exit(int exit_code) {
    /* Phase 23: SYS_EXIT now takes an exit code in EBX (previously
     * ignored) - explicitly passing 0 here rather than leaving EBX as
     * whatever it happened to contain, now that the kernel actually
     * records and can return this value via process_wait(). */
    __asm__ volatile ("int $0x80" : : "a"(SYS_EXIT), "b"(exit_code));
}

static bool bytes_equal(const char* a, const char* b, int len) {
    for (int i = 0; i < len; i++) {
        if (a[i] != b[i]) {
            return false;
        }
    }
    return true;
}

void sandbox_demo_task(void) {
    sys_write("\n[sandbox] Starting; my capability list only grants "
              "HELLO.TXT and the gateway (10.0.2.2).\n");

    /* Phase 70: userland/wm-rs/wm.rs's own --selftest mode - run
     * first, deliberately, before every other test in this task - see
     * that file's own run_selftest() doc comment for why a synthetic-
     * mouse-input self-test, run through the exact same real SYS_EXEC
     * path every other ELF test in this file already uses, is the
     * only practical way to automatically verify a real, interactive
     * program's own actual dragging/focus/snapping logic in this
     * project's existing headless test harness (which has no way to
     * inject real mouse movement into a running QEMU instance).
     * run_selftest() itself never calls sys_gfx_enter() or touches
     * real hardware state at all - exit code 0 means every one of its
     * five sub-cases passed. Placed here, first, rather than after
     * HELLO.ELF/HELLOC.ELF (where it originally lived) for a real,
     * honest reason: this project's own, separate, still-open
     * scheduling-corruption bug (PROGRESS.md) has repeatedly struck
     * this exact task right around that later point during this
     * phase's own testing, and confirmed (via directly removing this
     * exact call and observing the identical crash still occur) to be
     * completely unrelated to this new code - but running first, while
     * this task's own state is freshest, gives this real, new self-
     * test the best practical chance of actually executing and being
     * observed, rather than being just as likely to get silently
     * starved of a clean run as everything already known to sit in
     * that same, already-tracked blast radius. */
    const char* exec_argv_wm[] = {"WM.ELF", "--selftest"};
    int exec_pid_wm = sys_exec("WM.ELF", exec_argv_wm, 2);
    if (exec_pid_wm >= 0) {
        int exit_code_wm = sys_wait(exec_pid_wm);
        if (exit_code_wm == 0) {
            sys_write("[sandbox] PASS: WM.ELF --selftest - real window "
                      "dragging, click-to-focus, taskbar focus, and edge "
                      "snap/un-snap all behaved correctly.\n");
        } else {
            sys_write("[sandbox] FAIL: WM.ELF --selftest reported at "
                      "least one failing case.\n");
        }
    } else {
        sys_write("[sandbox] FAIL: SYS_EXEC(\"WM.ELF\") failed to "
                  "start.\n");
    }

    /* The allowed file: should succeed. */
    int h = sys_open("HELLO.TXT");
    if (h >= 0) {
        char buf[128];
        int n = sys_read(h, buf, sizeof(buf) - 1);
        if (n > 0) {
            buf[n] = '\0';
            sys_write("[sandbox] PASS: HELLO.TXT opened and read (allowed "
                      "by capability list): ");
            sys_write(buf);
        } else {
            sys_write("[sandbox] FAIL: HELLO.TXT was opened but SYS_READ "
                      "returned nothing.\n");
        }
        sys_close(h);
    } else {
        sys_write("[sandbox] FAIL: expected SYS_OPEN(\"HELLO.TXT\") to "
                  "succeed - it's in my capability list.\n");
    }

    /* The disallowed file: should be denied. */
    int h2 = sys_open("SYSTEM.CFG");
    if (h2 < 0) {
        sys_write("[sandbox] PASS: SYS_OPEN(\"SYSTEM.CFG\") correctly "
                  "denied - not in my capability list.\n");
    } else {
        sys_write("[sandbox] FAIL: SYSTEM.CFG should have been denied, "
                  "but SYS_OPEN succeeded!\n");
        sys_close(h2);
    }

    /* The allowed network destination (the gateway, 10.0.2.2): should
     * succeed. 0x0A000202 is 10.0.2.2 in host byte order - the same
     * encoding ip_make(10,0,2,2) produces (see kernel/net/net.h),
     * spelled out numerically here since this file can't reach that
     * kernel-side helper from ring 3. */
    int send_ok = sys_net_send(0x0A000202u, 9999,
                                "Hello from a sandboxed process!");
    if (send_ok == 0) {
        sys_write("[sandbox] PASS: SYS_NET_SEND to the gateway succeeded - "
                  "it's in my capability list.\n");
    } else {
        sys_write("[sandbox] FAIL: expected SYS_NET_SEND to the gateway to "
                  "succeed - it's in my capability list.\n");
    }

    /* The disallowed network destination: should be denied. */
    int send_denied = sys_net_send(0x0A000264u, 9999,
                                    "This should never be sent.");
    if (send_denied < 0) {
        sys_write("[sandbox] PASS: SYS_NET_SEND to 10.0.2.100 correctly "
                  "denied - not in my capability list.\n");
    } else {
        sys_write("[sandbox] FAIL: 10.0.2.100 should have been denied, but "
                  "SYS_NET_SEND succeeded!\n");
    }

    /* Spawn capability (allowed case - the denied case is exercised by
     * a separate, plainly-created process with no capabilities at
     * all; see kernel_main()). */
    int spawned_pid = sys_spawn();
    if (spawned_pid >= 0) {
        sys_write("[sandbox] PASS: SYS_SPAWN succeeded - spawn capability "
                  "was granted.\n");
    } else {
        sys_write("[sandbox] FAIL: expected SYS_SPAWN to succeed - spawn "
                  "capability was granted.\n");
    }

    /* Phase 23: load and run a real, independently-compiled ELF32
     * executable (tools/elf-fixtures/hello.asm), reusing the same
     * spawn capability. Passes two arguments and checks both that the
     * child's own output appears (proving argv really reached it) and
     * that SYS_WAIT returns its real exit code (42, a specific,
     * checkable value the ELF itself chose) rather than just "did it
     * start." */
    const char* exec_argv[] = {"HELLO.ELF", "hello-from-novaos"};
    int exec_pid = sys_exec("HELLO.ELF", exec_argv, 2);
    if (exec_pid >= 0) {
        int exit_code = sys_wait(exec_pid);
        if (exit_code == 42) {
            sys_write("[sandbox] PASS: SYS_EXEC loaded and ran a real ELF "
                      "executable - SYS_WAIT returned exit code 42 as "
                      "expected.\n");
        } else {
            sys_write("[sandbox] FAIL: HELLO.ELF exited with an "
                      "unexpected code.\n");
        }
    } else {
        sys_write("[sandbox] FAIL: SYS_EXEC(\"HELLO.ELF\") failed to "
                  "start.\n");
    }

    /* Phase 24: the same check, but for a real C program
     * (userland/examples/hello.c) linked against the minimal libc
     * port instead of hand-written assembly - proving printf(),
     * argv access, and malloc()/strcpy()/strcat()/free() all
     * genuinely work, not just raw syscalls. A distinct exit code (7,
     * vs. HELLO.ELF's 42) makes it possible to tell which test
     * produced which result in the boot log. */
    const char* exec_argv_c[] = {"HELLOC.ELF", "libc-test"};
    int exec_pid_c = sys_exec("HELLOC.ELF", exec_argv_c, 2);
    if (exec_pid_c >= 0) {
        int exit_code_c = sys_wait(exec_pid_c);
        if (exit_code_c == 7) {
            sys_write("[sandbox] PASS: SYS_EXEC loaded and ran a real C "
                      "program against the minimal libc - SYS_WAIT "
                      "returned exit code 7 as expected.\n");
        } else {
            sys_write("[sandbox] FAIL: HELLOC.ELF exited with an "
                      "unexpected code.\n");
        }
    } else {
        sys_write("[sandbox] FAIL: SYS_EXEC(\"HELLOC.ELF\") failed to "
                  "start.\n");
    }

    /* Phase 36: SYS_PIPE/SYS_WRITE_HANDLE, exercised through the real
     * ring-3 syscall path - proves the syscall-dispatch layer around
     * kernel/rust/pipe.rs works correctly (the Rust implementation
     * itself has its own, separate, more thorough direct-call
     * self-test - see kernel_main()). A standing, automated part of
     * this task's self-test suite - see tools/linker.ld's own comment
     * and PROGRESS.md for the real bug this test originally,
     * reproducibly exposed (nothing to do with pipes' own
     * correctness) and its fix, before which this exact test had to
     * be temporarily left out of the automated suite. */
    int pipe_handles[2] = {-1, -1};
    if (sys_pipe(pipe_handles) == 0) {
        int read_handle = pipe_handles[0];
        int write_handle = pipe_handles[1];
        static const char pipe_msg[] = "Hello through a NovaOS pipe!";
        const int pipe_msg_len = (int)sizeof(pipe_msg) - 1;

        int written = sys_write_handle(write_handle, pipe_msg, pipe_msg_len);
        char pipe_readback[64];
        int got = sys_read(read_handle, pipe_readback,
                            (int)sizeof(pipe_readback));
        sys_close(read_handle);
        sys_close(write_handle);

        if (written == pipe_msg_len && got == pipe_msg_len &&
            bytes_equal(pipe_readback, pipe_msg, pipe_msg_len)) {
            sys_write("[sandbox] PASS: SYS_PIPE/SYS_WRITE_HANDLE/SYS_READ - "
                      "wrote and read back the exact same message through "
                      "a pipe.\n");
        } else {
            sys_write("[sandbox] FAIL: pipe readback didn't match what was "
                      "written.\n");
        }
    } else {
        sys_write("[sandbox] FAIL: SYS_PIPE failed.\n");
    }

    /* Phase 27: prove fork() genuinely duplicates this process and
     * that copy-on-write correctly isolates the two copies - not just
     * that fork() runs without crashing, but that the actual process-
     * isolation contract holds: a write in the child must not be
     * visible to the parent's own copy of the same memory. Using a
     * plain stack local (not static/heap) deliberately exercises
     * COW on the exact stack this function is currently running on -
     * the most direct test of the mechanism, since fork() itself is
     * called from partway through it. */
    volatile int shared_value = 100;
    int fork_result = sys_fork();

    if (fork_result == 0) {
        /* Child: modify our own copy, then exit with a distinct,
         * checkable code the parent can verify via SYS_WAIT. */
        shared_value = 999;
        sys_write("[sandbox-child] I am the child - shared_value is "
                  "now 999 in my own copy.\n");
        sys_exit(55);
    } else if (fork_result > 0) {
        /* Parent: wait for the child, then check that OUR OWN copy of
         * shared_value is untouched - the actual COW correctness
         * test, not just "did fork() return a plausible pid." */
        int child_exit = sys_wait(fork_result);
        if (shared_value == 100 && child_exit == 55) {
            sys_write("[sandbox] PASS: fork() + copy-on-write correctly "
                      "isolated parent and child - my copy of "
                      "shared_value is still 100, child exited with "
                      "code 55 as expected.\n");
        } else {
            sys_write("[sandbox] FAIL: fork()/COW isolation broken.\n");
        }
    } else {
        sys_write("[sandbox] FAIL: SYS_FORK failed.\n");
    }

    /* Phase 48: authenticates against the real account persisted in
     * tools/fixtures/USERS.CFG (loaded at boot via
     * firstrun_check_and_run()'s own userscfg_load() call), not a
     * kernel-side hardcoded test account (Phase 47's original
     * approach) - proves the full persistence pipeline (a real file
     * on disk -> vfs_read_file -> userscfg_load -> rust_users_load ->
     * rust_users_authenticate), not just the in-memory add/
     * authenticate calls kernel/rust/users.rs's own self-test already
     * covers. Three checks, not one: this process starts as uid 0
     * (every sandboxed task does - see process.h's own comment); a
     * wrong password must fail *and* leave the uid unchanged; the
     * correct password must succeed and actually change the uid to
     * this fixture account's real value (700), not just return
     * success without the identity actually changing. */
    unsigned int uid_before = sys_getuid();

    int wrong_login = sys_login("persisted", "wrong-password-entirely");
    unsigned int uid_after_wrong = sys_getuid();

    int right_login = sys_login("persisted", "persisted-pw");
    unsigned int uid_after_right = sys_getuid();

    if (uid_before == 0 && wrong_login == -1 && uid_after_wrong == 0 &&
        right_login == 0 && uid_after_right == 700) {
        sys_write("[sandbox] PASS: SYS_LOGIN/SYS_GETUID - started as uid 0, "
                  "a wrong password was correctly rejected (uid unchanged), "
                  "the correct password (against the real account "
                  "persisted in USERS.CFG) succeeded and this process is "
                  "now uid 700.\n");
    } else {
        sys_write("[sandbox] FAIL: SYS_LOGIN/SYS_GETUID behaved "
                  "unexpectedly.\n");
    }

    /* Phase 51: SYS_SUDO, exercised through the real ring-3 syscall
     * path against two real, distinct accounts persisted in
     * USERS.CFG - not just kernel/rust/users.rs's own direct-call
     * self-test. This process is currently logged in as "persisted"
     * (uid 700, gid 700 - NOT the admin group) from the test just
     * above; sudo with that account's own genuinely correct password
     * must still be refused, proving authentication alone is not
     * authorization. Then logs in as "admin" (uid 800, gid 0 - the
     * admin group, but not already root) to prove the success path:
     * a wrong password refused, then the correct one actually
     * escalating this process to uid 0. */
    int sudo_denied_non_admin = sys_sudo("persisted-pw");
    unsigned int uid_after_denied_sudo = sys_getuid();

    int admin_login = sys_login("admin", "admin-correct-password");
    unsigned int uid_after_admin_login = sys_getuid();

    int sudo_wrong_password = sys_sudo("totally-the-wrong-password");
    unsigned int uid_after_wrong_sudo = sys_getuid();

    int sudo_success = sys_sudo("admin-correct-password");
    unsigned int uid_after_sudo = sys_getuid();

    if (sudo_denied_non_admin == -1 && uid_after_denied_sudo == 700 &&
        admin_login == 0 && uid_after_admin_login == 800 &&
        sudo_wrong_password == -1 && uid_after_wrong_sudo == 800 &&
        sudo_success == 0 && uid_after_sudo == 0) {
        sys_write("[sandbox] PASS: SYS_SUDO - a non-admin account's own "
                  "correct password was correctly refused (not in the "
                  "admin group), a wrong password for a real admin "
                  "account was refused, and the admin account's correct "
                  "password succeeded, genuinely escalating this process "
                  "to uid 0.\n");
    } else {
        sys_write("[sandbox] FAIL: SYS_SUDO behaved unexpectedly.\n");
    }

    sys_exit(0);

    /* Never reached: process_exit_current() (called by the SYS_EXIT
     * handler) marks this process TERMINATED and switches away
     * permanently. A plain busy-loop, not hlt, as a fallback: hlt is a
     * privileged instruction and would fault immediately at ring 3. */
    for (;;) { }
}
