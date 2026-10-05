#ifndef TASK_PROCESS_H
#define TASK_PROCESS_H

#include "../include/types.h"
#include "../arch/x86/cpu/isr.h"

/* Phase 75: the process table itself is now genuinely growable (see
 * kernel/rust/growtable.rs) rather than the flat, fixed-size
 * `process_t process_table[MAX_PROCESSES]` array this constant used
 * to size directly. History worth keeping: Phase 72 already raised
 * this same ceiling once, 16 -> 32, after a real, repeatedly-logged
 * "process table full" failure - this project's own boot-time test
 * sequence (multiple selftests, each exec'ing several real child
 * processes of their own) genuinely needed more slots than existed,
 * and that fix's own comment predicted needing to do this again:
 * "Simple headroom, not a structural fix... true slot recycling is a
 * separate, larger, riskier change deliberately not taken on here."
 * This phase is that structural fix for the CEILING (not for
 * recycling - see growtable.rs's own comment on why that stays a
 * separate, deliberately unaddressed question): the table now starts
 * at exactly this many slots (so boot-time behaviour and timing are
 * unchanged from before this phase) and grows one more chunk of this
 * same size at a time, on demand, instead of failing outright. */
#define PROCESS_TABLE_CHUNK_SIZE 32

/* How many chunks (see above) the process table may grow to -
 * PROCESS_TABLE_CHUNK_SIZE * PROCESS_TABLE_MAX_CHUNKS = 2048 total
 * process slots, ever, across the kernel's whole life (slots are
 * still never recycled - see growtable.rs). Real headroom over
 * anything this project's own test suite or demos come remotely
 * close to needing, not a tight fit - an explicit, honest, generous
 * bound rather than an unstated "unlimited" claim, matching this
 * project's own standing preference (MAX_EXEC_ARGS, MAX_CAPABILITIES,
 * MAX_SHARED_LIBS all make the identical choice). */
#define PROCESS_TABLE_MAX_CHUNKS 64
#define KERNEL_STACK_SIZE (16 * 1024)
/* Phase 71: doubled from 8KB to 16KB after directly observing a real
 * stack overflow - userland/novainit-rs/novainit.rs's own selftest
 * genuinely exhausted the previous 8KB budget, confirmed by a
 * distinctive, repeatable fault signature (eip landing inside the
 * user stack's own address range, at an address decodable as literal
 * string data rather than real code - a corrupted return address, not
 * a coincidence). Not a guess: this exact fix was verified to
 * resolve that exact, reproduced crash - see this phase's own
 * PROGRESS.md entry for the full, honest investigation. */
#define USER_STACK_SIZE   (16 * 1024)

/* Where a user task's private stack is mapped in its OWN address
 * space. Every user process uses the same virtual address for it -
 * that's fine, even desirable for keeping user_demo.c simple, because
 * each process has its own page directory (see paging_create_
 * address_space()): the same virtual address maps to a different,
 * private physical frame per process. This is the actual isolation
 * mechanism Phase 5 adds - see PROGRESS.md. Chosen well above the
 * shared 0-64MB kernel range so it can never collide with it. */
#define USER_STACK_VIRT_BASE 0x40000000u

/* Phase 24: where a process's heap (grown via SYS_SBRK) starts. Well
 * clear of typical ELF load addresses (elf.c's test fixtures load
 * around 0x08048000) and well below the user stack (0x40000000
 * above), with hundreds of megabytes of headroom on each side for a
 * program's own segments to grow without colliding - generous given
 * this kernel has no mechanism yet to detect or prevent two regions
 * growing into each other, an honest gap tracked in PROGRESS.md. */
#define HEAP_VIRT_BASE 0x20000000u

/* Phase 11: how many distinct filenames a sandboxed process can be
 * granted access to at creation time. Small and fixed - a real
 * capability system would want a dynamic, revocable grant mechanism;
 * this is deliberately just enough to prove the enforcement mechanism
 * works (see kernel/arch/x86/cpu/syscall.c's SYS_OPEN handler and
 * PROGRESS.md). */
#define MAX_CAPABILITIES 4

typedef enum {
    PROCESS_UNUSED = 0, /* slot is free */
    /* Phase 57: a new, transient state - a slot that allocate_slot()
     * (process.c) has just claimed, under process_table_lock, but
     * whose caller hasn't finished setting it up (and called
     * scheduler_add()/set it PROCESS_READY) yet. Exists specifically
     * to close a real two-CPU race: without it, two CPUs calling
     * allocate_slot() at the same physical instant could both scan
     * the table, both find the same UNUSED slot, and both claim it -
     * a real double-allocation of one process_t to two different
     * callers. Marking the slot ALLOCATING (not UNUSED, not READY)
     * the moment it's claimed - all still under the lock - makes the
     * second CPU's scan correctly skip it, the same way PROCESS_READY/
     * RUNNING/TERMINATED already made a slot ineligible for
     * allocate_slot() before this phase. Never itself picked by the
     * scheduler (kernel/task/scheduler.c's pick_next_locked() only
     * ever considers PROCESS_READY). If a caller fails after
     * allocate_slot() succeeds (e.g. process_exec_internal() finding
     * out-of-memory partway through), the slot is simply left
     * ALLOCATING forever - matching this kernel's existing, already-
     * documented "process slots are never actually recycled"
     * limitation (see process_wait()'s own comment), unchanged by
     * this phase. */
    PROCESS_ALLOCATING,
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

    /* Phase 30: a broader, explicit "may open any file" grant -
     * distinct from allowed_files[] (a small, fixed list for a
     * sandboxed process that only ever needs 1-2 specific files
     * known in advance). A general-purpose shell needs to open
     * whatever file the user names at runtime, which the fixed-list
     * model can't express - this is the honest, explicit extension
     * for that specific, narrow case rather than silently loosening
     * allowed_files[] itself. False by default, the same least-
     * privilege pattern as every other capability here; only granted
     * to the shell process itself, not to arbitrary exec'd programs
     * (see process_exec_with_any_file_access() in process.h). */
    bool can_open_any_file;

    /* Phase 23: the value passed to SYS_EXIT (or 0 if the process is
     * still running / was never given one). Meaningful once
     * process_wait() lets a caller retrieve it. */
    int exit_code;

    /* Phase 24: SYS_SBRK bookkeeping. heap_current is the logical
     * break (may sit mid-page); heap_mapped_end is how far pages have
     * actually been mapped so far (always page-aligned) - tracked
     * separately so process_sbrk() only maps the new pages an
     * increment actually needs, not the whole heap on every call.
     * Both start equal to HEAP_VIRT_BASE (an empty heap, nothing
     * mapped yet - the same "break exists but has size zero until
     * moved" convention real brk()/sbrk() use). */
    uint32_t heap_current;
    uint32_t heap_mapped_end;

    /* Phase 47: real process identity - UID 0 is "root" by
     * convention (matching Unix), the same convention any future
     * permission-checking code should rely on. Set only via
     * process_set_identity() (kernel-side, e.g. the initial boot
     * identity) or a successful SYS_LOGIN (ring-3, via real password
     * authentication against kernel/rust/users.rs's own database) -
     * never settable directly by an unprivileged syscall the way a
     * bare "setuid(any value)" would be, which would make this
     * meaningless as an identity check. Propagates across both
     * fork() (a child is still "the same user," matching real Unix)
     * and exec() (deliberately different from this kernel's existing,
     * separate capability model - allowed_files[]/allowed_hosts[]/
     * can_spawn reset to nothing on exec, by original design; uid/gid
     * represent *who is running this*, which does not change just
     * because a new program was loaded, matching real exec()
     * semantics - see process.c's own comments at each call site). */
    uint32_t uid;
    uint32_t gid;

    /* Phase 57: true only for this kernel's one permanently-idle
     * kernel task (see process_pin_to_bsp()'s own comment below and
     * kernel/init/main.c's idle_task_entry()) - keeps it off the AP
     * entirely. idle's own loop body is `hlt` waiting for the *shared*
     * PIT/IO-APIC timer tick to wake it and have scheduler_on_tick()
     * preempt it back out - that tick is still routed to the BSP only
     * (kernel/rust/apic.rs's ioapic_program_isa_redirects(), unchanged
     * since Phase 56), so an AP that ever picked up idle would `hlt`
     * forever waiting for an interrupt that can never reach it - a
     * real deadlock this flag exists specifically to prevent, not a
     * defensive nicety. False (0, via process_table[]'s own
     * zero-initialization in process_init()) for every other process,
     * so ordinary work is free to land on either CPU. */
    bool bsp_only;

    /* Phase 73: 1 when no CPU is executing on (or in the middle of
     * saving state into) this process any more; 0 from the moment the
     * scheduler picks it until the switch AWAY from it has finished.
     * do_schedule() marks the outgoing process READY under
     * scheduler_lock but has to release that lock BEFORE switch_context()
     * has saved the process's new kernel esp - so without this, the other
     * CPU could pick it in that gap and load its STALE saved esp, running
     * it from an old point on a stack the first CPU is still using (or,
     * for a process that had never been saved, a garbage esp: the
     * observed crash was `mov esp, <garbage>` inside switch_context).
     * switch_context() sets it to 1 the instant ESP leaves the old stack;
     * pick_next_locked() will not choose a READY process until then.
     * Newly created processes start at 1 (see process_publish()).
     * Placed last in the struct so no existing field's offset changes.
     * volatile: written by one CPU, read by another. */
    volatile uint32_t off_cpu;
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

/* Phase 59: see process.c's own comment on this function - a narrow,
 * deliberate exception used only to let a real ring-3 self-test task
 * genuinely exercise SYS_EXEC_TRUSTED's own capability-delegation
 * path, not a general substitute for process_create_sandboxed_task()
 * above. */
int process_create_sandboxed_task_trusted(const char* name,
                                           void (*entry)(void),
                                           const char** filenames,
                                           int file_count,
                                           const uint32_t* hosts,
                                           int host_count, bool can_spawn);

/* Phase 74: shared-library address-space layout. Every loaded shared
 * library gets a fixed SLOT of virtual address space, in DT_NEEDED
 * (breadth-first, cross-image) load order, starting at LIB_VIRT_BASE:
 * library 0 at LIB_VIRT_BASE, library 1 at LIB_VIRT_BASE +
 * LIB_SLOT_SIZE, and so on. This is a bump allocator over a handful of
 * fixed slots, not a general VMA allocator - the same level of
 * simplicity this kernel already uses for the heap (HEAP_VIRT_BASE,
 * grows up, never reused) and the user stack (one fixed region), and
 * enough for what a demo/test library actually needs.
 *
 * 0x10000000 sits comfortably between the executable's own load
 * address (0x08048000, see elf.c and every userland build.sh's
 * -Ttext) and HEAP_VIRT_BASE (0x20000000): MAX_SHARED_LIBS slots of
 * LIB_SLOT_SIZE each span at most 0x10000000..0x18000000, four times
 * over before it would ever reach the heap. LIB_SLOT_SIZE (4MB) is
 * far more than any library this project builds needs - real headroom
 * for growth, not a tight fit - and MAX_SHARED_LIBS (8) bounds the
 * total number of shared libraries one process may load, transitively
 * across the whole DT_NEEDED graph (the main executable's own needed
 * list, plus each library's own needed list in turn). Reject rather
 * than silently truncate if either limit would be exceeded - see
 * elf.c's own load_shared_libraries(). */
#define LIB_VIRT_BASE  0x10000000u
#define LIB_SLOT_SIZE  0x00400000u
#define MAX_SHARED_LIBS 8

/* Phase 23: how many argv entries process_exec() will pass through -
 * a small, fixed bound, the same "simple and honest about the limit"
 * choice as MAX_CAPABILITIES above rather than a dynamically-sized
 * list. */
#define MAX_EXEC_ARGS 8

/* Phase 73: the environment a process can hand to a child at exec time
 * (SYS_EXEC_ENV / SYS_EXEC_TRUSTED_ENV) - at most MAX_EXEC_ENV
 * "NAME=value" strings totalling at most MAX_EXEC_ENV_BYTES including
 * each string's NUL. Mirrored exactly by NOVA_ENV_MAX_VARS /
 * NOVA_ENV_MAX_BYTES in userland/libc/include/novasys.h, and enforced
 * by libc's own setenv()/putenv() too, so a program cannot build an
 * environment its own children would then be refused. The strings live
 * on the child's initial user stack, which is only USER_STACK_SIZE
 * (16KB) in total - hence MAX_EXEC_STACK_BYTES below, which caps
 * argv + envp (strings AND pointer arrays) together at half of it and
 * leaves the program at least 8KB of real stack. */
#define MAX_EXEC_ENV       32
#define MAX_EXEC_ENV_BYTES 4096
#define MAX_EXEC_STACK_BYTES (USER_STACK_SIZE / 2)

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

/* Phase 73: process_exec() plus an environment for the new process.
 * `envp` is an array of `envc` "NAME=value" strings (NULL/0 for none -
 * exactly what plain process_exec() passes); they are copied onto the
 * new process's initial stack in the standard position after argv, where
 * crt0.asm (userland/libc/crt0.asm) finds them and publishes them as
 * `environ`. Fails (-1, before anything is allocated) if envc exceeds
 * MAX_EXEC_ENV, the strings exceed MAX_EXEC_ENV_BYTES, or argv plus
 * envp together would not fit in MAX_EXEC_STACK_BYTES. */
int process_exec_env(const char* path, const char** argv, int argc,
                     const char** envp, int envc);

/* Phase 29: like process_exec(), but grants the new process access to
 * `filenames` (up to MAX_CAPABILITIES) before it ever runs - for
 * trusted, ring-0 callers only (the ordinary SYS_EXEC syscall never
 * reaches this). See process.c's comment on the function for the
 * full reasoning. */
int process_exec_with_files(const char* path, const char** argv, int argc,
                             const char** filenames, int file_count);

/* Phase 30: exec's a process with can_open_any_file granted (see that
 * field's comment above) AND can_spawn granted - the one genuinely
 * trusted, general-purpose program launched this way fundamentally
 * needs both broad file access and the ability to run other programs
 * (and, via SYS_EXEC_TRUSTED's own delegation - see kernel/arch/x86/
 * cpu/syscall.h - to pass that same trust on to whichever of ITS OWN
 * children it specifically chooses to), so both are covered by this
 * one trust decision. Kernel-boot-sequence use only; never reachable
 * from ring-3 SYS_EXEC.
 *
 * Phase 77: renamed from process_exec_as_shell() - the shell was
 * always just the one thing that happened to use this at the time,
 * never anything this function's own behavior was actually specific
 * to. What the kernel's boot sequence execs here (SYSTEM.CFG's own
 * init_path - see userland/shell/firstrun.c) is now this kernel's
 * real, permanent PID 1 (userland/novainit-rs/novainit.rs by default
 * - see that file's own module comment), not the shell directly; the
 * shell receives this exact same grant one hop later, by novainit's
 * own deliberate choice to launch it as a `trusted` supervised
 * service (tools/fixtures/SERVICES.CFG), the same as any other
 * genuinely trusted program this function's own callers might one day
 * choose to launch this way instead. */
int process_exec_as_init(const char* path, const char** argv, int argc);

/* Phase 59: SYS_EXEC_TRUSTED's own kernel-side implementation - see
 * kernel/arch/x86/cpu/syscall.h's own comment on that syscall for the
 * full reasoning. Like process_exec(), except the new process's
 * can_open_any_file is set to whatever the *calling* process's own
 * can_open_any_file currently is (read via process_current()), rather
 * than always false. Reachable from ring-3 SYS_EXEC_TRUSTED, unlike
 * process_exec_with_files()/process_exec_as_init() above - safe to
 * expose because an unprivileged caller's own can_open_any_file is
 * always false, so it has nothing to delegate; only a process already
 * carrying that grant (today, only the interactive shell) can pass it
 * on, and only to a program it explicitly execs this way, not to
 * everything it runs. process_current() being NULL (no calling
 * process) is treated as "nothing to delegate," the same
 * fail-closed default plain process_exec() already uses. */
int process_exec_trusted(const char* path, const char** argv, int argc);

/* Phase 73: process_exec_trusted() plus an environment - see
 * process_exec_env() above for the envp contract. */
int process_exec_trusted_env(const char* path, const char** argv, int argc,
                             const char** envp, int envc);

/* Blocks (yielding repeatedly) until process `pid` reaches
 * PROCESS_TERMINATED, then returns its exit code. Returns -1
 * immediately if no process with that pid currently exists in the
 * table at all (already reaped, or never existed) - see
 * PROGRESS.md's honest note on why this makes "wait for a pid that
 * already finished and was slotted over" a real, documented
 * limitation rather than a silently-wrong answer. */
int process_wait(int pid);

/* Phase 71: the non-blocking sibling process_wait() itself can't be -
 * kernel/task/svcinit.rs's own service supervisor needs to check on
 * several, independent children in the same pass without ever
 * blocking on one while the others also need attention, which process_
 * wait()'s own infinite retry-and-yield loop structurally can't do.
 * Checks `pid`'s own state exactly once, then returns immediately
 * either way - never yields, never loops.
 *
 * Returns `true` (with `*out_exit_code` set, and the same kernel-
 * stack/page-directory cleanup process_wait() itself does, so this is
 * a genuine reap, not just a peek) if `pid` had already reached
 * PROCESS_TERMINATED. Returns `false` otherwise - in which case
 * `*out_exists` (if non-NULL) distinguishes *why*: `true` means `pid`
 * is a real process that just hasn't terminated yet (the normal,
 * expected case while supervising a healthy service); `false` means
 * no such process exists in the table at all (never existed, or
 * already reaped by an earlier call) - a real, meaningful difference
 * process_wait() itself has always needed to tell apart too (see its
 * own comment on why a nonexistent pid returns -1 immediately rather
 * than blocking forever), just never had a way to report outward
 * until this function's own out-parameter gave it one. */
bool process_wait_nonblock(int pid, int* out_exit_code, bool* out_exists);

/* Phase 27: true fork() semantics - unlike process_exec() (Phase 23,
 * deliberately exec-style, not this), this genuinely duplicates the
 * calling process: a new process with its own address space, sharing
 * every existing page with the parent via copy-on-write (see
 * PAGE_COW in kernel/arch/x86/mm/paging.h) rather than eagerly
 * copying anything, and resuming execution at the exact point the
 * parent called SYS_FORK from - both processes continue as if they'd
 * both just returned from the same call, distinguished only by the
 * return value (0 in the child, the child's pid in the parent - the
 * real, standard Unix fork() contract). `parent_regs` is the
 * *calling* process's full saved register state at the moment of the
 * syscall (handed in directly from the syscall handler) - what makes
 * "resume where the parent was" possible at all. Returns the child's
 * pid, or -1 on failure (no free process slot, out of memory). */
int process_fork(registers_t* parent_regs);

/* Phase 24: grows (never shrinks - see below) the calling process's
 * heap by `increment` bytes, mapping fresh physical frames as needed,
 * and returns the *previous* break address - the same semantics
 * Unix's sbrk() has, so a userspace malloc() can use this exactly the
 * way a real one would. `increment` must be >= 0; shrinking the heap
 * back is not supported (freed memory is only reused within the
 * process's own malloc()/free(), never actually returned to the
 * kernel) - a real, documented limitation, not an oversight. Returns
 * (uint32_t)-1 on failure (negative increment, or out of physical
 * memory). */
uint32_t process_sbrk(process_t* p, int increment);

/* Marks the current process TERMINATED (recording `exit_code` for a
 * future process_wait() call) and switches away from it permanently.
 * Called by the SYS_EXIT syscall handler; never returns. */
void process_exit_current(int exit_code);

process_t* process_current(void);
process_t* process_table_entry(int index);

/* Phase 75: the process table's CURRENT capacity - how many indices
 * process_table_entry() will accept right now, not the fixed
 * PROCESS_TABLE_CHUNK_SIZE constant alone (the table starts at
 * exactly that many slots and grows in chunks of that size - see
 * process.h's own comment on it). scheduler.c's own round-robin scan
 * (pick_next_locked()) uses this instead of a compile-time constant,
 * the one place outside process.c that needs to know how large the
 * table currently is. */
int process_table_capacity(void);

/* Phase 47: sets a process's uid/gid directly - kernel-side only
 * (there is no syscall wrapper for this, deliberately; see
 * process.h's own comment on process_t's uid/gid fields for why an
 * unrestricted "set my own uid to anything" syscall would defeat the
 * point of having an identity at all). Used for the initial boot
 * identity (root, uid 0) and by process_login() below on successful
 * authentication - not intended for arbitrary runtime use. */
void process_set_identity(process_t* p, uint32_t uid, uint32_t gid);

/* Phase 47: the SYS_LOGIN syscall's actual implementation - verifies
 * `username`/`password` against kernel/rust/users.rs's own database
 * and, only on a real match, calls process_set_identity() on `p`
 * (the calling process). Returns true on success. The credential
 * check happens here, in the kernel, specifically so no ring-3
 * program can set its own identity without a real password - the
 * same reasoning a real login prompt's own privilege boundary
 * relies on. */
bool process_login(process_t* p, const char* username, const char* password);

/* Phase 51: the SYS_SUDO syscall's actual implementation - the real
 * privilege-escalation gate this kernel was missing entirely before
 * this phase (nothing anywhere checked "is this process uid 0" to
 * gate any action). Re-authenticates `p` (the calling process) by
 * looking up *its own current uid* in kernel/rust/users.rs's own
 * database (a process only knows its own numeric identity, not which
 * username it corresponds to - real sudo works the same way) and
 * checking `password` against that specific account. Escalates `p`
 * to uid 0/gid 0 only if the password is correct *and* that account
 * is a member of the "admin group" (gid == 0) - a correct password
 * for a non-admin account is refused, the actual point of this
 * function, not just password verification alone. Returns true only
 * on a real escalation. Deliberately scoped to escalating the calling
 * process itself for the rest of its own lifetime, not per-command
 * the way real sudo is - see kernel/task/sandbox_demo.c's own comment
 * and PROGRESS.md's Phase 51 "Known limitations" for the honest
 * account of why, and what a closer match to real sudo's scoping
 * would need. */
bool process_sudo(process_t* p, const char* password);

/* Phase 57: marks the process with this pid as never eligible to run
 * on an AP - see process_t's own `bsp_only` comment for exactly why
 * (today, this kernel's idle task specifically, and only that task).
 * Kernel-boot-sequence use only (kernel/init/main.c, once, right after
 * creating idle); no syscall wrapper exists or should - this is an
 * internal scheduling-safety mechanism, not a general-purpose CPU-
 * affinity feature. A no-op if `pid` doesn't currently exist. */
void process_pin_to_bsp(int pid);

/* Phase 81: deterministic regression test for the "Accessed bit makes a
 * shared kernel page table look process-owned" bug - see
 * paging_pde_is_kernel_shared() in paging.h and the test's own comment
 * in process.c. Returns true if fork-sharing and teardown both leave
 * the kernel's shared page tables alone. */
bool process_selftest_shared_pde_accessed_bit(void);

/* Phase 82: may `p`'s kernel stack and page directory be freed? True only
 * for a TERMINATED process whose CPU has finished switching off its kernel
 * stack (off_cpu). See the comment on the definition in process.c for the
 * intermittent use-after-free that the old "state == TERMINATED" test
 * allowed. */
bool process_is_reapable(const process_t* p);

/* Phase 83: true if `pid` names a process that exists and has not yet
 * exited. */
bool process_is_live(int pid);

/* Phase 82: deterministic test of that rule. */
bool process_selftest_reap_waits_for_the_exiting_cpu(void);

#endif
