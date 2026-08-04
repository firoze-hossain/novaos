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

static inline void sys_exit(void) {
    __asm__ volatile ("int $0x80" : : "a"(SYS_EXIT));
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

    sys_exit();

    /* Never reached: process_exit_current() (called by the SYS_EXIT
     * handler) marks this process TERMINATED and switches away
     * permanently. A plain busy-loop, not hlt, as a fallback: hlt is a
     * privileged instruction and would fault immediately at ring 3. */
    for (;;) { }
}
