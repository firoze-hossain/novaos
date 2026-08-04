/*
 * unprivileged_demo.c - proves SYS_SPAWN is denied by default (see
 * kernel/task/unprivileged_demo.h)
 */
#include "unprivileged_demo.h"
#include "../arch/x86/cpu/syscall.h"

static inline void sys_write(const char* str) {
    __asm__ volatile ("int $0x80" : : "a"(SYS_WRITE), "b"(str) : "memory", "cc");
}

static inline int sys_spawn(void) {
    int result = SYS_SPAWN;
    __asm__ volatile ("int $0x80" : "+a"(result) : : "memory", "cc");
    return result;
}

static inline void sys_exit(void) {
    __asm__ volatile ("int $0x80" : : "a"(SYS_EXIT));
}

void unprivileged_demo_task(void) {
    sys_write("\n[unprivileged] Starting; I have no capabilities at all "
              "(plain process_create_user_task).\n");

    int spawned_pid = sys_spawn();
    if (spawned_pid < 0) {
        sys_write("[unprivileged] PASS: SYS_SPAWN correctly denied - I was "
                  "never granted spawn capability.\n");
    } else {
        sys_write("[unprivileged] FAIL: SYS_SPAWN should have been denied, "
                  "but it succeeded!\n");
    }

    sys_exit();

    for (;;) { } /* never reached - see sandbox_demo.c's identical comment */
}
