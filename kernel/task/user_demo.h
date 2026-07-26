#ifndef TASK_USER_DEMO_H
#define TASK_USER_DEMO_H

/* Two ring-3 demo tasks, compiled into the kernel image and started
 * via process_create_user_task(). See the .c file and PROGRESS.md for
 * why "compiled in" rather than "loaded from disk" is the honest
 * current scope (no ELF loader yet) - the privilege separation AND
 * (as of Phase 5) the memory isolation they demonstrate are
 * nonetheless real and hardware-enforced.
 *
 * Both tasks use the same virtual stack address (USER_STACK_VIRT_BASE)
 * on purpose: each process has its own page directory, so the same
 * virtual address maps to a different, private physical frame per
 * process. Each task stamps its own stack with a distinct byte
 * pattern, yields the CPU several times (letting the other task run
 * in between), then verifies its own stack still reads back correctly
 * - proving process B never touched process A's memory, and vice
 * versa, despite both using identical virtual addresses. */
void user_demo_task_a(void);
void user_demo_task_b(void);

#endif
