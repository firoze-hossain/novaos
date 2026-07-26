#ifndef TASK_PROCESS_H
#define TASK_PROCESS_H

#include "../include/types.h"

#define MAX_PROCESSES 16
#define KERNEL_STACK_SIZE (8 * 1024)
#define USER_STACK_SIZE   (8 * 1024)

/* Where a user task's private stack is mapped in its OWN address
 * space. Every user process uses the same virtual address for it -
 * that's fine, even desirable for keeping user_demo.c simple, because
 * each process has its own page directory (see paging_create_
 * address_space()): the same virtual address maps to a different,
 * private physical frame per process. This is the actual isolation
 * mechanism Phase 5 adds - see PROGRESS.md. Chosen well above the
 * shared 0-64MB kernel range so it can never collide with it. */
#define USER_STACK_VIRT_BASE 0x40000000u

typedef enum {
    PROCESS_UNUSED = 0, /* slot is free */
    PROCESS_READY,
    PROCESS_RUNNING,
    PROCESS_TERMINATED,
} process_state_t;

typedef struct process {
    int pid;
    char name[32];
    process_state_t state;
    bool is_user;

    uint32_t esp;            /* saved stack pointer (valid when not RUNNING) */
    uint32_t kernel_stack_top; /* for TSS.esp0 when this task is running in ring3 */
    uint32_t page_directory_phys; /* CR3 value while this process runs */

    /* Only used for is_user tasks; kept so the slot can be freed. */
    void* kernel_stack_alloc;
} process_t;

/* Called once at boot, before any process_create_*() call. */
void process_init(void);

/* Creates a task that runs entirely in ring 0 with the given entry
 * point. Entry functions must not return (there is no completion
 * handling for kernel tasks yet - see PROGRESS.md); in practice both
 * of NovaOS's kernel tasks (idle, shell) already loop forever. */
int process_create_kernel_task(const char* name, void (*entry)(void));

/* Creates a task that starts in ring 3. `entry` is a function compiled
 * into the kernel image (see kernel/task/user_demo.c) rather than
 * loaded from disk - NovaOS has no ELF loader yet, so this is a
 * deliberately scoped-down way to exercise real ring-0/ring-3
 * privilege separation and the syscall path without one. The task
 * still runs at genuine CPU ring 3: it cannot execute privileged
 * instructions or access kernel-only memory, enforced by hardware, not
 * convention - it just got its starting address the easy way. See
 * PROGRESS.md for the honest scope. */
int process_create_user_task(const char* name, void (*entry)(void));

/* Marks the current process TERMINATED and switches away from it
 * permanently. Called by the SYS_EXIT syscall handler; never returns. */
void process_exit_current(void);

process_t* process_current(void);
process_t* process_table_entry(int index);

#endif
