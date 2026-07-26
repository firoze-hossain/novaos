#ifndef TASK_USER_DEMO_H
#define TASK_USER_DEMO_H

/* A ring-3 demo task, compiled into the kernel image and started via
 * process_create_user_task(). See its .c file and PROGRESS.md for why
 * "compiled in" rather than "loaded from disk" is the honest current
 * scope (no ELF loader yet) - the privilege separation it demonstrates
 * is nonetheless real and hardware-enforced. */
void user_demo_task(void);

#endif
