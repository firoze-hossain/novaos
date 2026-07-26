/*
 * user_demo.c - a task that actually runs at CPU ring 3
 *
 * This function is compiled into the kernel binary and its address is
 * used directly by process_create_user_task() - NovaOS has no ELF
 * loader yet (tracked for a future phase), so there's no "load an
 * executable from disk into its own address space" step here. What IS
 * real: this code executes with CPL=3, in the identity-mapped address
 * space shared with the kernel. It cannot call vga_puts() or any other
 * kernel function directly (there's nothing stopping the *compiler*
 * from letting you write that call, but the CPU would fault: ring 3
 * calling ring-0-only code isn't itself the fault - executing a
 * privileged instruction, or later touching descriptor-protected
 * kernel memory once real per-process isolation exists, is - so this
 * file is written to only ever talk to the kernel through `int 0x80`,
 * on purpose, to demonstrate the syscall boundary is the only path in.
 *
 * Ring 3 code must never execute a privileged instruction (hlt, cli,
 * sti, in/out, ...) - doing so takes an immediate #GP fault. The only
 * instruction this file uses to reach the kernel is INT, which is
 * explicitly permitted because syscall_init() set the int 0x80 gate's
 * DPL to 3.
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

void user_demo_task(void) {
    sys_write("\n[ring3] Hello from user mode! This was printed via the "
              "SYS_WRITE syscall, not a direct vga_puts() call.\n");

    for (volatile int i = 0; i < 3; i++) {
        sys_yield();
    }

    sys_write("[ring3] Demo task exiting via SYS_EXIT.\n");
    sys_exit();

    /* Never reached: process_exit_current() (called by the SYS_EXIT
     * handler) marks this process TERMINATED and switches away
     * permanently. A plain busy-loop, not hlt, as a fallback: hlt is a
     * privileged instruction and would fault immediately at ring 3. */
    for (;;) { }
}
