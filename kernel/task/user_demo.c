/*
 * user_demo.c - two tasks that run at CPU ring 3 and prove address-
 * space isolation between them
 *
 * These functions are compiled into the kernel binary and their
 * addresses are used directly by process_create_user_task() - NovaOS
 * has no ELF loader yet (tracked for a future phase), so there's no
 * "load an executable from disk into its own address space" step
 * here. What IS real: this code executes with CPL=3, in its OWN
 * per-process address space (see process.c / paging.c) rather than
 * the single shared space Phase 4 used. It cannot execute privileged
 * instructions, and as of Phase 5 it genuinely cannot read or corrupt
 * another process's private stack, even by guessing its virtual
 * address - because at the hardware level, that address simply isn't
 * mapped to the same physical memory in a different process's page
 * directory.
 *
 * Ring 3 code must never execute a privileged instruction (hlt, cli,
 * sti, in/out, ...) - doing so takes an immediate #GP fault. The only
 * instruction this file uses to reach the kernel is INT, explicitly
 * permitted because syscall_init() set the int 0x80 gate's DPL to 3.
 */
#include "user_demo.h"
#include "../arch/x86/cpu/syscall.h"

static inline void sys_write(const char* str) {
    __asm__ volatile ("int $0x80" : : "a"(SYS_WRITE), "b"(str) : "memory", "cc");
}

static inline void sys_yield(void) {
    __asm__ volatile ("int $0x80" : : "a"(SYS_YIELD) : "memory", "cc");
}

static inline void sys_exit(void) {
    __asm__ volatile ("int $0x80" : : "a"(SYS_EXIT));
}

static bool stack_reads_as(volatile char* buffer, char expected, int len) {
    for (int i = 0; i < len; i++) {
        if (buffer[i] != expected) {
            return false;
        }
    }
    return true;
}

#define STAMP_LEN 64

static void run_isolation_demo(char stamp, const char* start_msg,
                                const char* pass_msg, const char* fail_msg) {
    volatile char buffer[STAMP_LEN];
    for (int i = 0; i < STAMP_LEN; i++) {
        buffer[i] = stamp;
    }

    sys_write(start_msg);

    /* Yield several times so the scheduler round-robins through every
     * other task (idle, shell, and - the point of this exercise - the
     * other demo process) before checking whether this stack is still
     * intact. */
    for (volatile int i = 0; i < 5; i++) {
        sys_yield();
    }

    if (stack_reads_as(buffer, stamp, STAMP_LEN)) {
        sys_write(pass_msg);
    } else {
        sys_write(fail_msg);
    }

    sys_exit();

    /* Never reached: process_exit_current() (called by the SYS_EXIT
     * handler) marks this process TERMINATED and switches away
     * permanently. A plain busy-loop, not hlt, as a fallback: hlt is a
     * privileged instruction and would fault immediately at ring 3. */
    for (;;) { }
}

void user_demo_task_a(void) {
    run_isolation_demo(
        'A',
        "\n[ring3-A] Starting; stamped my private stack with 'A' x64.\n",
        "[ring3-A] PASS: stack still all 'A' after yielding 5x - "
        "process B never touched my private memory.\n",
        "[ring3-A] FAIL: stack corrupted! Address-space isolation is BROKEN.\n");
}

void user_demo_task_b(void) {
    run_isolation_demo(
        'B',
        "[ring3-B] Starting; stamped my private stack with 'B' x64 "
        "(same virtual address as process A, different physical page).\n",
        "[ring3-B] PASS: stack still all 'B' after yielding 5x - "
        "process A never touched my private memory.\n",
        "[ring3-B] FAIL: stack corrupted! Address-space isolation is BROKEN.\n");
}
