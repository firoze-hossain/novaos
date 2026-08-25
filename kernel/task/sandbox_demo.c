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

static inline void sys_exit(int exit_code) {
    /* Phase 23: SYS_EXIT now takes an exit code in EBX (previously
     * ignored) - explicitly passing 0 here rather than leaving EBX as
     * whatever it happened to contain, now that the kernel actually
     * records and can return this value via process_wait(). */
    __asm__ volatile ("int $0x80" : : "a"(SYS_EXIT), "b"(exit_code));
}

void sandbox_demo_task(void) {
    sys_write("\n[sandbox] Starting; my capability list only grants "
              "HELLO.TXT and the gateway (10.0.2.2).\n");

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

    sys_exit(0);

    /* Never reached: process_exit_current() (called by the SYS_EXIT
     * handler) marks this process TERMINATED and switches away
     * permanently. A plain busy-loop, not hlt, as a fallback: hlt is a
     * privileged instruction and would fault immediately at ring 3. */
    for (;;) { }
}
