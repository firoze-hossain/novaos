/*
 * greeter_task.c - the one fixed spawnable task type (see
 * kernel/task/greeter_task.h)
 */
#include "greeter_task.h"
#include "../arch/x86/cpu/syscall.h"

static inline void sys_write(const char* str) {
    __asm__ volatile ("int $0x80" : : "a"(SYS_WRITE), "b"(str) : "memory", "cc");
}

static inline void sys_exit(void) {
    __asm__ volatile ("int $0x80" : : "a"(SYS_EXIT));
}

void greeter_task(void) {
    sys_write("[greeter] Hello! I was spawned by another process via "
              "SYS_SPAWN.\n");
    sys_exit();

    /* Never reached: process_exit_current() (called by the SYS_EXIT
     * handler) marks this process TERMINATED and switches away
     * permanently. A plain busy-loop, not hlt, as a fallback: hlt is a
     * privileged instruction and would fault immediately at ring 3. */
    for (;;) { }
}
