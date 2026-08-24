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

/* Phase 11: how many distinct filenames a sandboxed process can be
 * granted access to at creation time. Small and fixed - a real
 * capability system would want a dynamic, revocable grant mechanism;
 * this is deliberately just enough to prove the enforcement mechanism
 * works (see kernel/arch/x86/cpu/syscall.c's SYS_OPEN handler and
 * PROGRESS.md). */
#define MAX_CAPABILITIES 4

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

    /* Phase 11: the exact set of filenames this process is allowed to
     * SYS_OPEN. Empty (allowed_file_count == 0) by default for every
     * process, including plain process_create_user_task() ones - a
     * process must be explicitly created via
     * process_create_sandboxed_task() to be granted anything at all.
     * Least privilege as the default, not an opt-out. */
    char allowed_files[MAX_CAPABILITIES][13];
    int allowed_file_count;

    /* Phase 14: same idea, extended to network destinations - the
     * exact set of IPv4 addresses (host byte order) this process is
     * allowed to SYS_NET_SEND to. Empty by default, same as file
     * capabilities. */
    uint32_t allowed_hosts[MAX_CAPABILITIES];
    int allowed_host_count;

    /* Phase 17: a third capability, this one just a boolean rather
     * than a list - whether this process may SYS_SPAWN at all. There
     * is currently only one spawnable task type (see
     * kernel/task/greeter_task.h), so a list of "which ones" would be
     * pointless; a fixed yes/no is the honest scope. False by default,
     * same least-privilege pattern as the other two. */
    bool can_spawn;

    /* Phase 23: the value passed to SYS_EXIT (or 0 if the process is
     * still running / was never given one). Meaningful once
     * process_wait() lets a caller retrieve it. */
    int exit_code;
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

/* Same as process_create_user_task(), but also grants the new process
 * three capabilities: the exact filenames (8.3, e.g. "HELLO.TXT") it
 * may SYS_OPEN, the exact IPv4 addresses (host byte order) it may
 * SYS_NET_SEND to, and whether it may SYS_SPAWN at all. Either list
 * may be empty (pass NULL/0 for the pair you don't need) - a process
 * only gets what's explicitly granted in whichever lists are
 * non-empty, plus can_spawn if true. Both lists are copied at
 * creation time, not referenced afterward; each count is clamped to
 * MAX_CAPABILITIES. */
int process_create_sandboxed_task(const char* name, void (*entry)(void),
                                   const char** filenames, int file_count,
                                   const uint32_t* hosts, int host_count,
                                   bool can_spawn);

/* Phase 23: how many argv entries process_exec() will pass through -
 * a small, fixed bound, the same "simple and honest about the limit"
 * choice as MAX_CAPABILITIES above rather than a dynamically-sized
 * list. */
#define MAX_EXEC_ARGS 8

/* Loads a real ELF32 executable from the filesystem and runs it as a
 * brand new process - NovaOS's answer to exec(), deliberately not
 * fork()+exec() as two separate steps. A true fork() (duplicating a
 * *running* process's entire address space) needs copy-on-write
 * memory management this kernel doesn't have yet; process_exec() is
 * closer to POSIX's posix_spawn() - create and load in one step -
 * which exists in POSIX for exactly this reason: most real callers
 * (a shell running a command) never needed true fork() semantics in
 * the first place. See PROGRESS.md for the full scope note.
 *
 * `path` is read via the VFS (an 8.3 filename, e.g. "HELLO.ELF").
 * `argv` is copied into the new process's own address space - the
 * caller's argv strings/pointers are not referenced afterward, so
 * they can safely live on the caller's own stack. Returns the new
 * process's pid, or -1 on failure (bad path, invalid ELF, out of
 * memory/process slots). */
int process_exec(const char* path, const char** argv, int argc);

/* Blocks (yielding repeatedly) until process `pid` reaches
 * PROCESS_TERMINATED, then returns its exit code. Returns -1
 * immediately if no process with that pid currently exists in the
 * table at all (already reaped, or never existed) - see
 * PROGRESS.md's honest note on why this makes "wait for a pid that
 * already finished and was slotted over" a real, documented
 * limitation rather than a silently-wrong answer. */
int process_wait(int pid);

/* Marks the current process TERMINATED (recording `exit_code` for a
 * future process_wait() call) and switches away from it permanently.
 * Called by the SYS_EXIT syscall handler; never returns. */
void process_exit_current(int exit_code);

process_t* process_current(void);
process_t* process_table_entry(int index);

#endif
