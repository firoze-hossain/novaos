#ifndef TASK_GREETER_TASK_H
#define TASK_GREETER_TASK_H

/* The one and only spawnable task type SYS_SPAWN can create (see
 * kernel/arch/x86/cpu/syscall.c's handle_spawn()). NovaOS has no
 * general exec-a-file-from-disk mechanism (no ELF loader, unchanged
 * since Phase 4), so there's no way for a syscall argument to name an
 * arbitrary program the way a real execve() path string would - a
 * fixed, single spawnable entry point is the honest scope until that
 * changes. Just prints a message and exits, enough to prove a spawned
 * process genuinely runs as its own process (its own PID, its own
 * address space) rather than merely returning success. */
void greeter_task(void);

#endif
