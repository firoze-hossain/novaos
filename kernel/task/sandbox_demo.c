/*
 * sandbox_demo.c - proves Phase 11's capability enforcement is real
 *
 * This task is created (see kernel_main()) with a capability list
 * containing only "HELLO.TXT". It deliberately tries to open both
 * that file (should succeed) and "SYSTEM.CFG" (should be denied,
 * since it's not in the granted list) and reports PASS/FAIL for each
 * - a falsifiable test, not a claim, the same approach Phase 5 used
 * to prove address-space isolation actually held.
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

static inline void sys_exit(void) {
    __asm__ volatile ("int $0x80" : : "a"(SYS_EXIT));
}

void sandbox_demo_task(void) {
    sys_write("\n[sandbox] Starting; my capability list only grants "
              "HELLO.TXT.\n");

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

    sys_exit();

    /* Never reached: process_exit_current() (called by the SYS_EXIT
     * handler) marks this process TERMINATED and switches away
     * permanently. A plain busy-loop, not hlt, as a fallback: hlt is a
     * privileged instruction and would fault immediately at ring 3. */
    for (;;) { }
}
