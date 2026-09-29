/*
 * process.c - process table storage and initial-stack construction
 *
 * Building the fake initial stack frame for a task that has never run
 * is the one genuinely tricky part of cooperative/preemptive task
 * switching done this way - see context_switch.asm's header comment
 * for the full picture. Short version: switch_context() restores
 * EFLAGS then EDI/ESI/EBX/EBP then `ret`s, so a new task's saved ESP
 * must point at a stack that looks exactly like switch_context() left
 * it mid-save, plus (for a user task) a fake IRET frame sitting right
 * after the fake return address.
 */
#include "process.h"
#include "scheduler.h"
#include "elf.h"
#include "../arch/x86/cpu/gdt.h"
#include "../arch/x86/cpu/isr.h"
#include "../arch/x86/mm/heap.h"
#include "../arch/x86/mm/paging.h"
#include "../arch/x86/mm/pmm.h"
#include "../fs/vfs.h"
#include "../lib/string.h"
#include "../lib/spinlock.h"
#include "../include/kernel.h"

extern void enter_usermode(void);
/* Phase 27: see syscall_stub.asm - the exact tail of ordinary syscall
 * return handling, which a fork()'d child's fake context-switch frame
 * points at as its "return address" so being scheduled in for the
 * first time resumes exactly like returning from the parent's fork()
 * call, with the child's own copy of every register. */
extern void syscall_return_point(void);

static process_t process_table[MAX_PROCESSES];
static int next_pid = 1;

/* Phase 57: named directly in this project's own release-readiness
 * roadmap ("process_table[]"). Guards exactly one thing:
 * allocate_slot()'s scan-then-claim of a free slot, the one genuinely
 * racy operation two CPUs could perform on this table at the same
 * physical instant (see PROCESS_ALLOCATING's own comment in
 * process.h for the exact race this closes). Every other read of
 * process_table[] in this file (process_wait()'s scan for a pid,
 * free_user_address_space()/cow_share_address_space() walking a
 * *specific* process's own page tables) either only ever touches a
 * process this same CPU already exclusively owns (the currently-
 * running one) or tolerates a benign, eventually-consistent stale
 * read (process_wait()'s poll loop just tries again) - deliberately
 * not locked, to keep this addition as narrow as the actual race,
 * not a single big table-wide lock for every access. next_pid itself
 * is only ever incremented from inside a slot a caller already holds
 * exclusively (after allocate_slot() returns, before anything else
 * can see that slot) in every existing call site, so it doesn't
 * independently need this lock either - see PROGRESS.md for the
 * honest note that a *very* unlucky simultaneous pid assignment
 * (extremely unlikely given every call site pairs it with a real
 * kmalloc()/pmm_alloc_frame() first, both now genuinely serializing
 * via their own locks in practice) remains a narrow, undocumented-
 * until-now edge this specific lock doesn't close - a real, if minor,
 * honest gap, not silently swept under the rug. */
static spinlock_t process_table_lock;

static void copy_name(char* dest, const char* src, size_t dest_size) {
    size_t i = 0;
    while (src[i] != '\0' && i < dest_size - 1) {
        dest[i] = src[i];
        i++;
    }
    dest[i] = '\0';
}

void process_init(void) {
    memset(process_table, 0, sizeof(process_table));
    spinlock_init(&process_table_lock);
}

static process_t* allocate_slot(void) {
    uint32_t flags = spinlock_acquire(&process_table_lock);
    for (int i = 0; i < MAX_PROCESSES; i++) {
        if (process_table[i].state == PROCESS_UNUSED) {
            /* Claim it immediately, still under the lock, so a second
             * CPU calling allocate_slot() concurrently can never see
             * this same slot as UNUSED too - see PROCESS_ALLOCATING's
             * own comment in process.h for the full account of the
             * race this closes.
             *
             * pid assignment moved here, Sep 2026: a real, confirmed
             * bug, not the "extremely unlikely" edge this function's
             * own prior comment dismissed it as. Every call site used
             * to call allocate_slot() (locked) and only afterward,
             * separately, execute `p->pid = next_pid++` completely
             * unlocked - the prior reasoning ("every call site pairs
             * it with a real kmalloc()/pmm_alloc_frame() first, both
             * genuinely serializing") does not actually close the
             * race: serializing on a *different* lock does not
             * prevent two different CPUs, each already holding their
             * own, separate, legitimately-allocated slot, from then
             * both reaching the unlocked `next_pid++` at the same
             * physical instant and reading the identical value before
             * either one's increment lands - producing two different,
             * concurrently-running processes with the same pid.
             * Confirmed as the actual, real explanation for a
             * persistent CI failure, not theorized: sys_wait() on a
             * freshly-exec'd pid was observed reading back the wrong
             * exit code, exactly what pid collision predicts and what
             * memory-corruption theories investigated earlier could
             * not fully explain. Assigning the pid here, still inside
             * this same critical section, makes it atomic with the
             * slot claim itself - the two can never again be split
             * across an unlocked gap. */
            process_table[i].pid = next_pid++;
            process_table[i].state = PROCESS_ALLOCATING;
            spinlock_release(&process_table_lock, flags);
            return &process_table[i];
        }
    }
    spinlock_release(&process_table_lock, flags);
    return NULL;
}

/* Phase 73: the LAST step of every process-creation path.
 *
 * A process must not become visible to the scheduler (state READY) until
 * every field the scheduler and the context switch read is fully
 * initialised: esp, kernel_stack_top, page_directory_phys, and - for the
 * capability tests - allowed_files/can_spawn/can_open_any_file. This
 * kernel runs a second CPU whose idle loop polls the scheduler
 * continuously, so it can pick up a READY process within nanoseconds of
 * that state being written. Every creation path used to write
 * `state = PROCESS_READY` FIRST and the rest afterwards; an AP that
 * chose the new process in that gap loaded a stale/zero esp and a zero
 * page_directory_phys (a bad CR3 in paging_switch_address_space() - a
 * triple fault, QEMU exiting early), or ran the task while its real
 * capabilities had not been granted yet (spurious "capability denied").
 * Until published the slot stays PROCESS_ALLOCATING (set by
 * allocate_slot()), which pick_next_locked() never returns. The
 * compiler barrier keeps the state store after every store above it;
 * x86 does not reorder stores with other stores, so that is sufficient
 * for the other CPU to observe them in order. */
static void process_publish(process_t* p) {
    p->off_cpu = 1; /* not on any CPU: see process_t.off_cpu */
    __asm__ volatile ("" ::: "memory");
    p->state = PROCESS_READY;
    scheduler_add(p);
}

int process_create_kernel_task(const char* name, void (*entry)(void)) {
    process_t* p = allocate_slot();
    if (p == NULL) {
        kernel_log("[FAULT] process_create_kernel_task: process table full\n");
        return -1;
    }

    void* kstack = kmalloc(KERNEL_STACK_SIZE);
    if (kstack == NULL) {
        kernel_log("[FAULT] process_create_kernel_task: out of memory\n");
        return -1;
    }

    uint32_t kstack_top = (uint32_t)kstack + KERNEL_STACK_SIZE;
    uint32_t* sp = (uint32_t*)kstack_top;

    *(--sp) = (uint32_t)entry; /* switch_context's `ret` jumps straight in */
    *(--sp) = 0; /* ebp */
    *(--sp) = 0; /* ebx */
    *(--sp) = 0; /* esi */
    *(--sp) = 0; /* edi */
    *(--sp) = 0x202; /* eflags: IF=1, reserved bit 1 = 1 */

    /* pid already assigned atomically by allocate_slot() itself. */
    copy_name(p->name, name, sizeof(p->name));
    p->is_user = false;
    p->esp = (uint32_t)sp;
    p->kernel_stack_top = kstack_top;
    p->kernel_stack_alloc = kstack;
    p->page_directory_phys = paging_kernel_directory_phys();
    p->allowed_file_count = 0;
    p->allowed_host_count = 0;
    p->can_spawn = false;
    p->can_open_any_file = false;
    p->exit_code = 0;
    p->heap_current = HEAP_VIRT_BASE;
    p->heap_mapped_end = HEAP_VIRT_BASE;
    p->uid = 0; /* kernel-created tasks run as root (uid 0) - trusted,
                   internal, never the result of a login */
    p->gid = 0;

    process_publish(p);
    return p->pid;
}

/* Shared setup for both process_create_user_task() and
 * process_create_sandboxed_task() - everything except the capability
 * list, which the two callers fill in differently (empty vs. granted).
 * Returns the new process with state already READY and already
 * registered with the scheduler; the caller only needs to set
 * allowed_files/allowed_file_count and return p->pid. */
static process_t* create_user_task_common(const char* name,
                                           void (*entry)(void)) {
    process_t* p = allocate_slot();
    if (p == NULL) {
        kernel_log("[FAULT] process_create_user_task: process table full\n");
        return NULL;
    }

    void* kstack = kmalloc(KERNEL_STACK_SIZE);
    if (kstack == NULL) {
        kernel_log("[FAULT] process_create_user_task: out of memory (kstack)\n");
        return NULL;
    }

    /* Unlike the kernel stack (only ever touched by kernel code on
     * this process's behalf), the user stack is what actually needs
     * to be private per process - see USER_STACK_VIRT_BASE's comment
     * in process.h. It's backed by fresh PMM frames and mapped only
     * into this process's own page directory, not kmalloc'd from the
     * shared heap arena the way Phase 4 did it (every process's
     * directory maps that whole shared range, so a "private" stack
     * living there wouldn't have been private at all). */
    uint32_t address_space = paging_create_address_space();
    uint32_t* pd = (uint32_t*)address_space;

    const int stack_pages = USER_STACK_SIZE / 4096;
    for (int i = 0; i < stack_pages; i++) {
        uint32_t frame = pmm_alloc_frame();
        if (frame == 0) {
            kernel_log("[FAULT] process_create_user_task: out of memory "
                       "(user stack frame %d/%d)\n", i + 1, stack_pages);
            return NULL;
        }
        uint32_t virt = USER_STACK_VIRT_BASE + (uint32_t)i * 4096u;
        paging_map_page(pd, virt, frame,
                         PAGE_PRESENT | PAGE_WRITE | PAGE_USER);
    }
    uint32_t ustack_top = USER_STACK_VIRT_BASE + USER_STACK_SIZE;

    uint32_t kstack_top = (uint32_t)kstack + KERNEL_STACK_SIZE;
    uint32_t* sp = (uint32_t*)kstack_top;

    /* Fake IRET frame `enter_usermode` will consume. Pushed in reverse
     * (SS first) so EIP ends up as the lowest/first of these five,
     * matching the order `iret` pops them in. */
    *(--sp) = GDT_USER_DATA;         /* SS  */
    *(--sp) = ustack_top;            /* ESP - the process's own private mapping */
    *(--sp) = 0x202;                 /* EFLAGS: IF=1 */
    *(--sp) = GDT_USER_CODE;         /* CS  */
    *(--sp) = (uint32_t)entry;       /* EIP */

    *(--sp) = (uint32_t)enter_usermode; /* switch_context's `ret` target */
    *(--sp) = 0; /* ebp */
    *(--sp) = 0; /* ebx */
    *(--sp) = 0; /* esi */
    *(--sp) = 0; /* edi */
    *(--sp) = 0x202; /* eflags */

    /* pid already assigned atomically by allocate_slot() itself. */
    copy_name(p->name, name, sizeof(p->name));
    p->is_user = true;
    p->esp = (uint32_t)sp;
    p->kernel_stack_top = kstack_top;
    p->kernel_stack_alloc = kstack;
    p->page_directory_phys = address_space;
    p->allowed_file_count = 0; /* least privilege by default */
    p->allowed_host_count = 0;
    p->can_spawn = false;
    p->can_open_any_file = false;
    p->exit_code = 0;
    p->heap_current = HEAP_VIRT_BASE;
    p->heap_mapped_end = HEAP_VIRT_BASE;
    p->uid = 0; /* kernel-created tasks (this function backs both
                   process_create_user_task() and
                   process_create_sandboxed_task(), both only ever
                   called directly by kernel_main() at boot, never as
                   the result of a login) run as root (uid 0) - see
                   process.h's own comment on process_t's uid/gid */
    p->gid = 0;

    return p;
}

int process_create_user_task(const char* name, void (*entry)(void)) {
    process_t* p = create_user_task_common(name, entry);
    if (p == NULL) {
        return -1;
    }
    process_publish(p);
    return p->pid;
}

int process_create_sandboxed_task(const char* name, void (*entry)(void),
                                   const char** filenames, int file_count,
                                   const uint32_t* hosts, int host_count,
                                   bool can_spawn) {
    process_t* p = create_user_task_common(name, entry);
    if (p == NULL) {
        return -1;
    }

    if (file_count > MAX_CAPABILITIES) {
        file_count = MAX_CAPABILITIES;
    }
    for (int i = 0; i < file_count; i++) {
        copy_name(p->allowed_files[i], filenames[i],
                  sizeof(p->allowed_files[i]));
    }
    p->allowed_file_count = file_count;

    if (host_count > MAX_CAPABILITIES) {
        host_count = MAX_CAPABILITIES;
    }
    for (int i = 0; i < host_count; i++) {
        p->allowed_hosts[i] = hosts[i];
    }
    p->allowed_host_count = host_count;

    p->can_spawn = can_spawn;
    p->can_open_any_file = false; /* sandboxed tasks only ever use the
                                      fixed allowed_files[] list, never
                                      this broader grant */

    process_publish(p);
    return p->pid;
}

/* Phase 59: the one deliberate, narrow exception to the invariant
 * documented just above - exists only to let a real, kernel-compiled
 * ring-3 self-test task genuinely exercise SYS_EXEC_TRUSTED's own
 * capability-delegation path (process_exec_trusted(), see process.h's
 * comment on it) as a real ring-3 caller making a real `int 0x80`
 * call, the same way any actual delegation-capable program eventually
 * would - rather than only testing process_exec_trusted() indirectly
 * or not at all. Everything else is identical to process_create_
 * sandboxed_task() above; this is not a general-purpose replacement
 * for it and should not become one - a sandboxed task's whole point is
 * the narrow, explicit allowed_files[] list, and can_open_any_file
 * should stay reserved for the interactive shell (via process_exec_
 * as_shell()) and whatever it explicitly delegates to, not become
 * something every sandboxed test task can casually ask for. */
int process_create_sandboxed_task_trusted(const char* name,
                                           void (*entry)(void),
                                           const char** filenames,
                                           int file_count,
                                           const uint32_t* hosts,
                                           int host_count, bool can_spawn) {
    process_t* p = create_user_task_common(name, entry);
    if (p == NULL) {
        return -1;
    }

    if (file_count > MAX_CAPABILITIES) {
        file_count = MAX_CAPABILITIES;
    }
    for (int i = 0; i < file_count; i++) {
        copy_name(p->allowed_files[i], filenames[i],
                  sizeof(p->allowed_files[i]));
    }
    p->allowed_file_count = file_count;

    if (host_count > MAX_CAPABILITIES) {
        host_count = MAX_CAPABILITIES;
    }
    for (int i = 0; i < host_count; i++) {
        p->allowed_hosts[i] = hosts[i];
    }
    p->allowed_host_count = host_count;

    p->can_spawn = can_spawn;
    p->can_open_any_file = true;

    process_publish(p);
    return p->pid;
}

/* Phase 22: frees the resources a ring-3 process exclusively owns -
 * the physical frames backing its private user stack, the page table
 * that mapped them, and the process's own page directory. Never
 * touches the shared kernel range (identity-mapped 0-64MB, copied
 * into every process's directory by paging_create_address_space()) -
 * only this process's own exclusive allocations, found by walking
 * just the one page-directory entry USER_STACK_VIRT_BASE always falls
 * into, which nothing else ever shares. This closes a real, long-
 * documented gap: every user process's stack memory, page table, and
 * page directory leaked permanently on exit since Phase 4/5. */
/* Frees every page-table frame and physical page this process
 * privately owns in its own address space, then the page directory
 * itself. Originally written (Phase 22) knowing only about the one
 * fixed user-stack location; generalized here (Phase 23) to walk
 * every one of the 1024 page-directory entries and free whichever
 * ones differ from the shared kernel template, since ELF segments
 * (elf.c) can now be mapped at any address, not just the user stack's
 * fixed one - a comparison against the actual kernel directory is
 * used rather than hardcoding an index boundary, so this stays
 * correct even if the kernel's own identity-mapped range ever
 * changes size. The shared range itself (compares equal to the
 * kernel's own template entries) is never touched - only entries this
 * process itself added via paging_map_page() are ever freed. */
static void free_user_address_space(uint32_t page_directory_phys) {
    uint32_t* pd = (uint32_t*)page_directory_phys;
    uint32_t* kernel_pd = (uint32_t*)paging_kernel_directory_phys();

    for (uint32_t i = 0; i < 1024; i++) {
        if (!(pd[i] & PAGE_PRESENT)) {
            continue;
        }
        if (pd[i] == kernel_pd[i]) {
            continue; /* shared kernel entry - not this process's to free */
        }

        uint32_t pt_phys = pd[i] & 0xFFFFF000u;
        uint32_t* pt = (uint32_t*)pt_phys;
        for (uint32_t j = 0; j < 1024; j++) {
            if (!(pt[j] & PAGE_PRESENT)) {
                continue;
            }
            /* A real, previously-undiscovered bug, found and fixed
             * here (not the already-documented "COW frames leak"
             * limitation in PROGRESS.md - this is a worse, different
             * problem than a leak): a page still marked PAGE_COW may
             * still be actively mapped and in use by another process
             * - the parent this one was fork()'d from, or a sibling
             * that shares the same original frame. Freeing it here
             * unconditionally, as this code previously did, hands
             * that exact physical frame to the PMM's free list while
             * that other process's own page tables still point at it
             * as present, valid memory - the next unrelated
             * pmm_alloc_frame() call anywhere in the kernel can then
             * hand the same physical frame to something completely
             * different, which promptly overwrites live code/data/
             * stack contents the other process still depends on.
             * Confirmed as the real mechanism behind a hang this
             * project had been treating as a mysterious, timing-
             * sensitive scheduler bug: instrumented the scheduler
             * directly and traced the exact failure to a process
             * being scheduled immediately after a fork()'d sibling
             * exited and freed a still-shared page this way. Without
             * this fix, a COW frame is never freed at all - a real,
             * bounded leak (documented in PROGRESS.md's own Phase 27
             * notes), not a correctness bug - which is the safe,
             * conservative fallback until real reference counting on
             * shared frames exists (tracked as its own follow-up). */
            if (pt[j] & PAGE_COW) {
                continue;
            }
            pmm_free_frame(pt[j] & 0xFFFFF000u);
        }
        pmm_free_frame(pt_phys);
    }

    pmm_free_frame(page_directory_phys);
}

/* Phase 27: walks the parent's address space the same way
 * free_user_address_space() does (skipping the shared kernel range
 * via direct comparison against the kernel template, not a hardcoded
 * boundary), but instead of freeing anything, gives the child its own
 * page tables that point at the *same* physical data frames as the
 * parent - real copy-on-write sharing, not an eager copy. Both the
 * parent's and the child's page table entries for every shared frame
 * are marked PAGE_COW and have PAGE_WRITE cleared; whichever process
 * writes to a shared page first gets its own private copy via
 * paging.c's page fault handler, the other keeps sharing the original
 * until (if ever) it also writes to it. Returns false on allocation
 * failure, leaving the child's address space partially built - the
 * caller treats that as a fork() failure and doesn't schedule the
 * child, but see PROGRESS.md for the leaked-partial-state caveat that
 * implies. */
static bool cow_share_address_space(uint32_t parent_pd_phys,
                                     uint32_t child_pd_phys) {
    uint32_t* parent_pd = (uint32_t*)parent_pd_phys;
    uint32_t* child_pd = (uint32_t*)child_pd_phys;
    uint32_t* kernel_pd = (uint32_t*)paging_kernel_directory_phys();

    for (uint32_t i = 0; i < 1024; i++) {
        if (!(parent_pd[i] & PAGE_PRESENT)) {
            continue;
        }
        if (parent_pd[i] == kernel_pd[i]) {
            continue; /* shared kernel entry - paging_create_address_
                          space() already gave the child this same
                          entry; nothing to do */
        }

        uint32_t* parent_pt = (uint32_t*)(parent_pd[i] & 0xFFFFF000u);

        uint32_t child_pt_frame = pmm_alloc_frame();
        if (child_pt_frame == 0) {
            return false;
        }
        uint32_t* child_pt = (uint32_t*)child_pt_frame;
        memset(child_pt, 0, 4096);

        for (uint32_t j = 0; j < 1024; j++) {
            if (!(parent_pt[j] & PAGE_PRESENT)) {
                continue;
            }

            uint32_t frame = parent_pt[j] & 0xFFFFF000u;
            uint32_t cow_flags = PAGE_PRESENT | PAGE_USER | PAGE_COW;

            parent_pt[j] = frame | cow_flags; /* the parent's own
                                                   mapping becomes
                                                   read-only+COW too -
                                                   both sides must
                                                   fault to get a
                                                   private copy */
            child_pt[j] = frame | cow_flags;
        }

        child_pd[i] = child_pt_frame | PAGE_PRESENT | PAGE_WRITE | PAGE_USER;
    }

    /* The parent's own currently-loaded page directory just had some
     * of its entries changed from writable to read-only - without
     * this, the CPU's TLB could still have stale "writable" entries
     * cached for pages the parent had recently touched, letting it
     * keep writing without ever faulting into the COW handler.
     * Reloading CR3 with its own value flushes all non-global TLB
     * entries without actually switching address spaces. */
    uint32_t cr3 = parent_pd_phys;
    __asm__ volatile ("mov %0, %%cr3" : : "r"(cr3) : "memory");

    return true;
}

void process_exit_current(int exit_code) {
    process_t* p = scheduler_current();
    if (p != NULL) {
        p->exit_code = exit_code;
        p->state = PROCESS_TERMINATED;
        kernel_log("[ OK ] Process '%s' (pid %d) exited with code %d\n",
                   p->name, p->pid, exit_code);

        /* A real, confirmed use-after-free bug used to be here: this
         * function runs ON the exiting process's own kernel stack
         * (reached via the SYS_EXIT syscall handler, still executing
         * on that same stack) - kfree()'ing p->kernel_stack_alloc at
         * this exact point frees the very memory the CPU is currently
         * using for local variables and return addresses, which the
         * heap allocator can then hand out to any other allocation
         * that happens to run before this function's own remaining
         * code (including scheduler_yield() itself, just below)
         * finishes using it - silently corrupting this stack's
         * contents out from under itself. Confirmed directly, not
         * theorized: forcing -smp 1 did NOT make the resulting crashes
         * (a different fault type and address on nearly every run -
         * General Protection Fault, Invalid Opcode at eip values as
         * implausibly low as 0x7, page faults at addresses that
         * decode as fragments of nearby ASCII strings) go away,
         * ruling out an SMP race and pointing directly at genuine
         * memory corruption instead - exactly what a live stack being
         * freed and reused produces.
         *
         * free_user_address_space() used to be called here too, and
         * was first assumed safe (it frees a *different* mapping than
         * the kernel stack, the reasoning went) - that assumption was
         * wrong, found on further investigation after the kernel-stack
         * fix alone reduced but did not eliminate the same class of
         * crash: that function's own last step frees the page
         * directory's own physical frame, which for a user process is
         * still this exact CPU's actively loaded CR3 at this point in
         * execution - handing that frame back to the PMM while it's
         * still translating every memory access this code (and
         * scheduler_yield() right after it) makes. Both frees now
         * happen together in process_wait() below, once it observes
         * PROCESS_TERMINATED - at that point this process has already
         * reached scheduler_yield() and can never be scheduled again
         * (so its page directory is never reloaded into CR3 again
         * either), and process_wait() itself runs on its *caller's*
         * own stack and own, different, already-active page
         * directory - not this one, either way. */
    }
    scheduler_yield();
    /* Should never reach here - a TERMINATED process is never picked
     * again - but fail safe rather than fall into undefined behavior.
     *
     * A real, confirmed bug used to be here: this looped on `hlt`
     * alone, on the assumption that scheduler_yield() always actually
     * switches away once a process is TERMINATED. On the BSP that's
     * true - the idle task is always eligible there, so
     * pick_next_locked() never comes back empty. On an AP it isn't:
     * idle is deliberately bsp_only (see process_t's own comment -
     * idle's hlt loop only ever wakes via the BSP-only timer
     * interrupt, so it can never safely run on an AP), and
     * pick_next_locked() skips bsp_only processes for AP callers. If
     * this exiting process's own AP has no *other* eligible task
     * ready at this exact moment, do_schedule() finds nothing, does
     * not switch, and simply returns - right back here, into a
     * process that's already PROCESS_TERMINATED. hlt on an AP then
     * waits forever for a timer interrupt that, on this kernel, is
     * never routed there at all - a genuine, silent, unrecoverable
     * hang, not a crash (nothing faults; there is simply nothing left
     * that will ever run). scheduler_ap_join() already established
     * the correct fix for this identical situation (an AP with
     * nothing yet eligible to run): busy-spin with `pause`, retrying
     * the scheduler, rather than ever halting - reused here instead
     * of duplicated differently. */
    for (;;) {
        scheduler_yield();
        __asm__ volatile ("pause" ::: "memory");
    }
}

process_t* process_table_entry(int index) {
    if (index < 0 || index >= MAX_PROCESSES) {
        return NULL;
    }
    return &process_table[index];
}

process_t* process_current(void) {
    return scheduler_current();
}

void process_pin_to_bsp(int pid) {
    /* Simple linear scan, unlocked - matches process_wait()'s own scan
     * (see process_table_lock's comment above for why this specific
     * read doesn't need it): called exactly once, at boot, before the
     * scheduler or any AP has started (kernel/init/main.c calls this
     * right after creating idle, both well before scheduler_start()),
     * so there is no other CPU that could be concurrently mutating
     * process_table[] at the time this runs. */
    for (int i = 0; i < MAX_PROCESSES; i++) {
        if (process_table[i].pid == pid && process_table[i].state != PROCESS_UNUSED) {
            process_table[i].bsp_only = true;
            return;
        }
    }
}

/* Writes `len` bytes into a (possibly not physically contiguous)
 * address space at `dest_vaddr`, one physical frame at a time - the
 * same walk-the-page-tables technique elf.c uses to copy segment
 * data, needed here because process_exec()'s argv/argc stack area
 * spans pages backed by independently-allocated PMM frames that
 * aren't guaranteed physically adjacent to each other. */
static void write_to_address_space(uint32_t* pd, uint32_t dest_vaddr,
                                    const void* src, uint32_t len) {
    const uint8_t* src_bytes = (const uint8_t*)src;
    uint32_t remaining = len;
    uint32_t vaddr = dest_vaddr;
    uint32_t src_pos = 0;

    while (remaining > 0) {
        uint32_t page_base = vaddr & 0xFFFFF000u;
        uint32_t page_offset = vaddr - page_base;
        uint32_t pd_index = page_base >> 22;
        uint32_t pt_index = (page_base >> 12) & 0x3FFu;
        uint32_t* pt = (uint32_t*)(pd[pd_index] & 0xFFFFF000u);
        uint32_t frame = pt[pt_index] & 0xFFFFF000u;

        uint32_t chunk = 4096 - page_offset;
        if (chunk > remaining) {
            chunk = remaining;
        }
        memcpy((void*)(frame + page_offset), src_bytes + src_pos, chunk);

        remaining -= chunk;
        vaddr += chunk;
        src_pos += chunk;
    }
}

/* Phase 57: guards the one shared, mutable static buffer below
 * (`elf_buffer`) - a real hazard this project's own release-readiness
 * roadmap didn't name specifically (it's not one of the four examples
 * given - process_table[]/the PMM bitmap/the heap allocator/"every
 * driver's own state" - but it's exactly the same *kind* of hazard,
 * found by reading this function while auditing those). Before this
 * phase, two CPUs both calling process_exec()/process_exec_with_
 * files()/process_exec_as_shell() at the same physical instant would
 * both read their own (different) ELF file into the *same* 2MB
 * buffer, corrupting whichever load loses the race - a real bug now
 * that Phase 56 made a second core able to run ring-3 code
 * concurrently with the first, not previously possible on one core.
 * Held across the whole read-validate-load sequence (vfs_read_file()
 * through elf_load()), all of which is bounded, non-blocking kernel
 * work with no scheduler_yield() anywhere inside it - safe to hold a
 * spinlock across, unlike do_schedule()'s own switch_context() (see
 * that function's own comment for why *that* one specifically must
 * never be called with a lock held). */
static spinlock_t exec_lock;

/* ------------------------------------------------------------------
 * Phase 74: dynamic linking. The pure ELF32 dynamic-section/
 * relocation logic lives in kernel/rust/dynlink.rs (see that file's
 * own module comment for the full design - eager/BIND_NOW binding,
 * which relocation types are supported and why, what's deliberately
 * out of scope); everything here is the OS-integration side that
 * module deliberately stays out of: reading a needed library's file
 * off the VFS, choosing where in the process's address space it
 * loads, mapping its pages, and driving the breadth-first walk over
 * its (transitive) DT_NEEDED graph.
 * ------------------------------------------------------------------ */

/* Field-for-field mirrors of kernel/rust/dynlink.rs's own #[repr(C)]
 * types - see that file for what each field means. Kept in this .c
 * file rather than a shared header for the same reason every other
 * Rust<->C boundary in this codebase is: a raw `extern` declaration
 * right where it's used (see kernel/fs/fat32.c's rust_journal_begin(),
 * for the established precedent), not a maintained shared header. */
typedef struct {
    const uint8_t* data;
    uint32_t size;
    uint32_t bias;
    uint32_t dyn_offset;
    uint32_t dyn_filesz;
    const elf_load_seg_t* segs;
    uint32_t seg_count;
} dynlink_image_info_t;

#define DYNLINK_MAX_NAME_LEN 64
typedef struct {
    uint8_t bytes[DYNLINK_MAX_NAME_LEN];
    uint32_t len;
} dynlink_needed_name_t;

typedef uint32_t (*dynlink_resolve_fn)(void* ctx, const uint8_t* name,
                                        uint32_t name_len);
typedef bool (*dynlink_write_fn)(void* ctx, uint32_t vaddr, uint32_t value);

extern int32_t rust_dynlink_get_needed(const uint8_t* data, uint32_t size,
                                        uint32_t dyn_offset,
                                        uint32_t dyn_filesz,
                                        const elf_load_seg_t* segs,
                                        uint32_t seg_count,
                                        dynlink_needed_name_t* out,
                                        uint32_t out_len);
extern uint32_t rust_dynlink_find_symbol(const dynlink_image_info_t* img,
                                          const uint8_t* name,
                                          uint32_t name_len);
extern int32_t rust_dynlink_apply_relocations(
    const dynlink_image_info_t* img, dynlink_resolve_fn resolve,
    void* resolve_ctx, dynlink_write_fn write, void* write_ctx);

static const char* dynlink_error_str(int32_t rc) {
    switch (rc) {
        case -1: return "malformed ELF/.dynamic data";
        case -2: return "an undefined symbol could not be resolved";
        case -3: return "a relocation targeted unmapped memory";
        case -4: return "needs an unsupported R_386_COPY relocation (a "
                         "directly-referenced mutable global data symbol "
                         "from a shared library) - see kernel/rust/"
                         "dynlink.rs";
        case -5: return "an unsupported relocation type";
        default: return "unknown error";
    }
}

/* A single 4-byte word write, into a page directory that may not
 * (should not, for a well-formed relocation, but this checks rather
 * than assumes) have every target address mapped. Unlike
 * write_to_address_space() above - used only for argv/envp strings,
 * always within the freshly-mapped user stack, so unconditionally
 * present - a relocation's target is computed from file data this
 * loader trusts but does not independently re-verify, so this walks
 * the page directory defensively and returns false on the first
 * not-present entry rather than dereferencing a garbage "frame"
 * address the way an unchecked walk would. This is exactly the
 * WriteFn contract kernel/rust/dynlink.rs's own apply_relocations()
 * expects: false means "stop, something is wrong," not "skip and
 * continue." */
static bool write_word_checked(uint32_t* pd, uint32_t vaddr, uint32_t value) {
    uint32_t page_base = vaddr & 0xFFFFF000u;
    uint32_t page_offset = vaddr - page_base;
    if (page_offset > 4096u - 4u) {
        /* A relocation's 4-byte target straddling a page boundary
         * never happens in practice (Elf32_Rel targets are always
         * naturally 4-byte aligned, per the ABI) - treated as
         * malformed input, not silently split across two writes. */
        return false;
    }
    uint32_t pd_index = page_base >> 22;
    uint32_t pt_index = (page_base >> 12) & 0x3FFu;
    if (!(pd[pd_index] & PAGE_PRESENT)) {
        return false;
    }
    uint32_t* pt = (uint32_t*)(pd[pd_index] & 0xFFFFF000u);
    if (!(pt[pt_index] & PAGE_PRESENT)) {
        return false;
    }
    uint32_t frame = pt[pt_index] & 0xFFFFF000u;
    *(uint32_t*)(frame + page_offset) = value;
    return true;
}

/* One loaded shared library: its own file bytes (in one of
 * lib_buffers[] below), where it landed in the process's address
 * space, and what elf_inspect() learned about it. */
typedef struct {
    char name[13]; /* 8.3 + NUL - matches userland/libc's own FILENAME_MAX */
    uint8_t* data;
    uint32_t size;
    uint32_t bias;
    elf_image_t image;
} loaded_lib_t;

/* Everything the resolve callback (dynlink_resolve_cb below) needs to
 * search: the main executable plus every library loaded so far, in
 * load order. A pointer to one of these is threaded through
 * rust_dynlink_apply_relocations() as its opaque resolve_ctx. */
typedef struct {
    const uint8_t* main_data;
    uint32_t main_size;
    const elf_image_t* main_image;
    loaded_lib_t libs[MAX_SHARED_LIBS];
    uint32_t lib_count;
} dynlink_world_t;

/* Real ELF global-scope symbol resolution searches the main
 * executable itself before any library (so a library can call back
 * into a symbol the executable defines - a real, if uncommon, pattern
 * on any Unix system), then each loaded library in DT_NEEDED load
 * order, stopping at the first match - exactly what this does. */
static uint32_t dynlink_resolve_cb(void* ctx, const uint8_t* name,
                                    uint32_t name_len) {
    dynlink_world_t* w = (dynlink_world_t*)ctx;

    dynlink_image_info_t main_img = {
        .data = w->main_data, .size = w->main_size, .bias = 0,
        .dyn_offset = w->main_image->dyn_offset,
        .dyn_filesz = w->main_image->dyn_filesz,
        .segs = w->main_image->segs, .seg_count = w->main_image->seg_count,
    };
    uint32_t addr = rust_dynlink_find_symbol(&main_img, name, name_len);
    if (addr != 0) {
        return addr;
    }

    for (uint32_t i = 0; i < w->lib_count; i++) {
        loaded_lib_t* lib = &w->libs[i];
        dynlink_image_info_t img = {
            .data = lib->data, .size = lib->size, .bias = lib->bias,
            .dyn_offset = lib->image.dyn_offset,
            .dyn_filesz = lib->image.dyn_filesz,
            .segs = lib->image.segs, .seg_count = lib->image.seg_count,
        };
        addr = rust_dynlink_find_symbol(&img, name, name_len);
        if (addr != 0) {
            return addr;
        }
    }
    return 0;
}

static bool dynlink_write_cb(void* ctx, uint32_t vaddr, uint32_t value) {
    return write_word_checked((uint32_t*)ctx, vaddr, value);
}

/* Phase 74: bounded, static storage for every shared library a single
 * exec() call loads - never on this function's own kernel stack
 * (KERNEL_STACK_SIZE is only 16KB, see process.h). Guarded by
 * exec_lock, the same reasoning as elf_buffer below: shared, global,
 * reused-across-calls storage that a second CPU execing concurrently
 * must not be able to race on. 512KB per library is real headroom -
 * every shared library this project's own toolchain has produced so
 * far is well under 20KB - not a tight fit against what a small,
 * genuinely useful shared library needs. */
#define LIB_FILE_BUFFER_SIZE (512u * 1024u)
static uint8_t lib_buffers[MAX_SHARED_LIBS][LIB_FILE_BUFFER_SIZE];

/* Loads every (transitive) DT_NEEDED shared library the main
 * executable needs, maps them into `page_directory_phys`, and
 * resolves every relocation - the main executable's own and every
 * loaded library's. Called with exec_lock already held (see
 * process_exec_internal below): every buffer this touches (`main_data`
 * - process_exec_internal's own elf_buffer - and lib_buffers[]) is
 * exactly the kind of shared, global, reused-across-calls static
 * storage exec_lock already exists to protect, so its scope simply
 * extends to cover this too rather than needing a second lock.
 *
 * Returns true on success. On any failure, logs the specific reason
 * (a missing library file, a DT_NEEDED graph wider or deeper than
 * MAX_SHARED_LIBS, an unresolved symbol, an unsupported relocation)
 * and returns false - process_exec_internal aborts the whole exec()
 * exactly as it already does for a malformed main ELF, releasing
 * every resource it had already allocated. */
static bool load_and_link_shared_libraries(const char* exec_path,
                                            const uint8_t* main_data,
                                            uint32_t main_size,
                                            const elf_image_t* main_image,
                                            uint32_t page_directory_phys) {
    dynlink_world_t world;
    world.main_data = main_data;
    world.main_size = main_size;
    world.main_image = main_image;
    world.lib_count = 0;

    /* Breadth-first queue of library names still needing to be loaded
     * - a plain array walked with a read index, not a separate queue
     * structure, since MAX_SHARED_LIBS already bounds it tightly. */
    char queue[MAX_SHARED_LIBS][13];
    uint32_t queue_len = 0;
    uint32_t queue_pos = 0;

    dynlink_needed_name_t names[MAX_SHARED_LIBS];
    int32_t n = rust_dynlink_get_needed(
        main_data, main_size, main_image->dyn_offset, main_image->dyn_filesz,
        main_image->segs, main_image->seg_count, names, MAX_SHARED_LIBS);
    if (n < 0) {
        kernel_log("[FAULT] process_exec('%s'): malformed .dynamic "
                   "section\n", exec_path);
        return false;
    }
    if ((uint32_t)n > MAX_SHARED_LIBS) {
        kernel_log("[FAULT] process_exec('%s'): needs %d shared libraries, "
                   "more than the %d this kernel supports\n", exec_path, n,
                   MAX_SHARED_LIBS);
        return false;
    }
    for (int32_t i = 0; i < n; i++) {
        if (names[i].len == 0 || names[i].len > 12) {
            kernel_log("[FAULT] process_exec('%s'): a DT_NEEDED name is "
                       "empty or longer than this filesystem's 8.3 "
                       "limit\n", exec_path);
            return false;
        }
        memcpy(queue[queue_len], names[i].bytes, names[i].len);
        queue[queue_len][names[i].len] = '\0';
        queue_len++;
    }

    while (queue_pos < queue_len) {
        const char* name = queue[queue_pos++];

        /* A diamond dependency (two libraries both needing a third) -
         * already loaded, don't load or relocate it twice. */
        bool already = false;
        for (uint32_t i = 0; i < world.lib_count; i++) {
            if (strcmp(world.libs[i].name, name) == 0) {
                already = true;
                break;
            }
        }
        if (already) {
            continue;
        }

        if (world.lib_count >= MAX_SHARED_LIBS) {
            kernel_log("[FAULT] process_exec('%s'): the DT_NEEDED graph "
                       "needs more than %d shared libraries total\n",
                       exec_path, MAX_SHARED_LIBS);
            return false;
        }

        uint32_t slot = world.lib_count;
        loaded_lib_t* lib = &world.libs[slot];
        uint32_t name_len = (uint32_t)strlen(name);
        memcpy(lib->name, name, name_len + 1);

        int file_size = vfs_read_file(name, lib_buffers[slot],
                                       LIB_FILE_BUFFER_SIZE);
        if (file_size <= 0) {
            kernel_log("[FAULT] process_exec('%s'): needed library '%s' "
                       "could not be read\n", exec_path, name);
            return false;
        }
        lib->data = lib_buffers[slot];
        lib->size = (uint32_t)file_size;

        if (!elf_inspect(lib->data, lib->size, &lib->image)) {
            kernel_log("[FAULT] process_exec('%s'): '%s' is not a valid "
                       "ELF32 shared library this loader supports\n",
                       exec_path, name);
            return false;
        }
        if (!lib->image.is_dyn) {
            kernel_log("[FAULT] process_exec('%s'): '%s' is not a shared "
                       "library (not ET_DYN - link it with -shared)\n",
                       exec_path, name);
            return false;
        }
        if (!lib->image.has_dynamic) {
            kernel_log("[FAULT] process_exec('%s'): '%s' has no "
                       "PT_DYNAMIC segment\n", exec_path, name);
            return false;
        }

        lib->bias = LIB_VIRT_BASE + slot * LIB_SLOT_SIZE;
        if (!elf_load_segments_biased(lib->data, lib->size,
                                       page_directory_phys, lib->bias,
                                       &lib->image)) {
            kernel_log("[FAULT] process_exec('%s'): failed to load '%s' "
                       "into memory\n", exec_path, name);
            return false;
        }

        world.lib_count = slot + 1;

        /* This library's own DT_NEEDED entries join the same queue -
         * real, transitive dependency resolution, not just one level
         * deep. */
        int32_t ln = rust_dynlink_get_needed(
            lib->data, lib->size, lib->image.dyn_offset,
            lib->image.dyn_filesz, lib->image.segs, lib->image.seg_count,
            names, MAX_SHARED_LIBS);
        if (ln < 0) {
            kernel_log("[FAULT] process_exec('%s'): '%s' has a malformed "
                       ".dynamic section\n", exec_path, name);
            return false;
        }
        for (int32_t i = 0; i < ln; i++) {
            if (queue_len >= MAX_SHARED_LIBS) {
                kernel_log("[FAULT] process_exec('%s'): the DT_NEEDED "
                           "graph is wider than %d entries\n", exec_path,
                           MAX_SHARED_LIBS);
                return false;
            }
            if (names[i].len == 0 || names[i].len > 12) {
                kernel_log("[FAULT] process_exec('%s'): '%s' has an "
                           "invalid DT_NEEDED name\n", exec_path, name);
                return false;
            }
            memcpy(queue[queue_len], names[i].bytes, names[i].len);
            queue[queue_len][names[i].len] = '\0';
            queue_len++;
        }
    }

    /* Every image involved (the main executable and every loaded
     * library) is now fully mapped, so every relocation's target
     * address is real, present memory - process the main executable's
     * own relocations, then each library's, in load order (the order
     * between images doesn't affect correctness - see
     * rust_dynlink_apply_relocations()'s own comment). */
    uint32_t* pd = (uint32_t*)page_directory_phys;

    dynlink_image_info_t main_img = {
        .data = main_data, .size = main_size, .bias = 0,
        .dyn_offset = main_image->dyn_offset,
        .dyn_filesz = main_image->dyn_filesz,
        .segs = main_image->segs, .seg_count = main_image->seg_count,
    };
    int32_t rc = rust_dynlink_apply_relocations(
        &main_img, dynlink_resolve_cb, &world, dynlink_write_cb, pd);
    if (rc != 0) {
        kernel_log("[FAULT] process_exec('%s'): relocating the executable "
                   "itself failed (%s)\n", exec_path, dynlink_error_str(rc));
        return false;
    }

    for (uint32_t i = 0; i < world.lib_count; i++) {
        loaded_lib_t* lib = &world.libs[i];
        dynlink_image_info_t img = {
            .data = lib->data, .size = lib->size, .bias = lib->bias,
            .dyn_offset = lib->image.dyn_offset,
            .dyn_filesz = lib->image.dyn_filesz,
            .segs = lib->image.segs, .seg_count = lib->image.seg_count,
        };
        rc = rust_dynlink_apply_relocations(&img, dynlink_resolve_cb, &world,
                                             dynlink_write_cb, pd);
        if (rc != 0) {
            kernel_log("[FAULT] process_exec('%s'): relocating '%s' "
                       "failed (%s)\n", exec_path, lib->name,
                       dynlink_error_str(rc));
            return false;
        }
    }

    /* %d, not %u: kernel/lib/stdio.c's own vsnprintf() - a much more
     * minimal, kernel-only implementation than userland/libc's Phase
     * 73 one - only recognises %s/%d/%x/%c. lib_count is always a
     * small non-negative count (0..MAX_SHARED_LIBS), so the signed/
     * unsigned distinction never actually matters here. */
    kernel_log("[ OK ] process_exec('%s'): dynamically linked, %d shared "
               "librar%s loaded\n", exec_path, (int)world.lib_count,
               world.lib_count == 1 ? "y" : "ies");
    return true;
}

/* Phase 71: grant_spawn is deliberately a separate parameter from
 * grant_any_file, not folded into it - process_exec_trusted() below
 * needs to delegate can_spawn independently of can_open_any_file,
 * since a real, legitimate caller can have one without the other
 * (userland/novainit-rs/'s own service supervisor, exec'd from within
 * a sandboxed task that has real, explicitly-granted can_spawn - see
 * kernel/task/sandbox_demo.c's own sandbox_caps - but no file access
 * beyond its own, narrow, explicit allow-list at all). Tying spawn
 * delegation to file-access delegation would have meant either
 * granting broad file access no caller here actually needs just to
 * unlock spawn delegation, or not being able to delegate spawn at
 * all without it - neither is the real, least-privilege grant this
 * situation actually calls for. */
static int process_exec_internal(const char* path, const char** argv,
                                  int argc, const char** filenames,
                                  int file_count, bool grant_any_file,
                                  bool grant_spawn, const char** envp,
                                  int envc) {
    if (argc > MAX_EXEC_ARGS) {
        argc = MAX_EXEC_ARGS;
    }

    /* Phase 73: refuse an environment (or argv+environment) that will
     * not fit, BEFORE anything is allocated. This has to come first,
     * not at the point the stack is written: the failure paths further
     * down (after allocate_slot()) do not hand their process-table slot
     * back, so a late "too big" exit would burn a slot every time -
     * exactly the failure mode that once exhausted the 16-slot table
     * (see PROGRESS.md, Phase 71). Every loop here is bounded by
     * MAX_EXEC_ENV / MAX_EXEC_ENV_BYTES, never by what the caller's
     * memory happens to contain. */
    if (envp == NULL || envc < 0) {
        envc = 0;
    }
    if (envc > MAX_EXEC_ENV) {
        kernel_log("[WARN] process_exec: environment has %d entries, more "
                   "than the %d allowed\n", envc, MAX_EXEC_ENV);
        return -1;
    }
    uint32_t env_bytes = 0;
    for (int i = 0; i < envc; i++) {
        uint32_t len = 0;
        while (envp[i][len] != '\0') {
            len++;
            if (len > MAX_EXEC_ENV_BYTES) {
                break; /* already over the limit - stop scanning */
            }
        }
        env_bytes += len + 1;
        if (env_bytes > MAX_EXEC_ENV_BYTES) {
            kernel_log("[WARN] process_exec: environment exceeds %d bytes\n",
                       MAX_EXEC_ENV_BYTES);
            return -1;
        }
    }
    /* argv had no size limit at all before this phase; it and the
     * environment now share the initial stack, so bound them together
     * (strings plus the argc/argv/envp/NULL words that point at them). */
    uint32_t stack_need = env_bytes + 4u * (uint32_t)(argc + envc + 3);
    for (int i = 0; i < argc; i++) {
        stack_need += (uint32_t)strlen(argv[i]) + 1;
        if (stack_need > MAX_EXEC_STACK_BYTES) {
            break;
        }
    }
    if (stack_need > MAX_EXEC_STACK_BYTES) {
        kernel_log("[WARN] process_exec: argv + environment need more than "
                   "%d bytes of initial stack\n", MAX_EXEC_STACK_BYTES);
        return -1;
    }

    /* 2MB - large enough for statically-linked Rust binaries (Phase
     * 33), which are substantially bigger than this project's C test
     * binaries even for trivial programs, since core's compiled code
     * (much of it unused by any single program) gets linked in
     * wholesale rather than as a shared library. Originally 64KB,
     * sized only for small C test binaries - a real ELF32 loader
     * would stream this rather than buffer the whole file at once,
     * which vfs_read_file() doesn't support yet - see PROGRESS.md. */
    static uint8_t elf_buffer[2 * 1024 * 1024];

    uint32_t exec_flags = spinlock_acquire(&exec_lock);

    int file_size = vfs_read_file(path, elf_buffer, sizeof(elf_buffer));
    if (file_size <= 0) {
        spinlock_release(&exec_lock, exec_flags);
        kernel_log("[FAULT] process_exec: couldn't read '%s'\n", path);
        return -1;
    }
    if (!elf_validate(elf_buffer, (uint32_t)file_size)) {
        spinlock_release(&exec_lock, exec_flags);
        kernel_log("[FAULT] process_exec: '%s' is not a valid ELF32 "
                   "executable this loader supports\n", path);
        return -1;
    }

    process_t* p = allocate_slot();
    if (p == NULL) {
        spinlock_release(&exec_lock, exec_flags);
        kernel_log("[FAULT] process_exec: process table full\n");
        return -1;
    }

    void* kstack = kmalloc(KERNEL_STACK_SIZE);
    if (kstack == NULL) {
        spinlock_release(&exec_lock, exec_flags);
        kernel_log("[FAULT] process_exec: out of memory (kstack)\n");
        return -1;
    }

    uint32_t address_space = paging_create_address_space();
    uint32_t* pd = (uint32_t*)address_space;

    uint32_t entry_point;
    if (!elf_load(elf_buffer, (uint32_t)file_size, address_space,
                   &entry_point)) {
        spinlock_release(&exec_lock, exec_flags);
        kernel_log("[FAULT] process_exec: failed to load '%s'\n", path);
        kfree(kstack);
        free_user_address_space(address_space);
        return -1;
    }

    /* Phase 74: dynamic linking. elf_inspect() re-examines the same
     * bytes elf_load() just finished loading - purely to learn whether
     * this binary has a PT_DYNAMIC segment, which elf_load() itself
     * never looks for (see elf.h - elf_load() is deliberately left
     * untouched so every existing static binary's load path carries
     * zero risk from this). A binary elf_inspect() itself can't
     * characterize (more than MAX_LOAD_SEGS PT_LOAD segments - not
     * true of anything this project's own toolchain has ever produced)
     * is simply treated as having no PT_DYNAMIC and run as a plain
     * static binary, exactly as it would have before this phase. */
    elf_image_t main_image;
    if (elf_inspect(elf_buffer, (uint32_t)file_size, &main_image) &&
        main_image.has_dynamic) {
        if (!load_and_link_shared_libraries(path, elf_buffer,
                                             (uint32_t)file_size,
                                             &main_image, address_space)) {
            spinlock_release(&exec_lock, exec_flags);
            kfree(kstack);
            free_user_address_space(address_space);
            return -1;
        }
    }

    /* elf_buffer itself is no longer touched past this point - what's
     * left builds the new process's own stack/register state from
     * already-copied-out data (entry_point, and argv/argc which never
     * came from elf_buffer at all), so the lock is released here
     * rather than held for the rest of this (fairly long) function -
     * no correctness reason to serialize the parts that don't share
     * anything. */
    spinlock_release(&exec_lock, exec_flags);

    const int stack_pages = USER_STACK_SIZE / 4096;
    for (int i = 0; i < stack_pages; i++) {
        uint32_t frame = pmm_alloc_frame();
        if (frame == 0) {
            kernel_log("[FAULT] process_exec: out of memory (user stack "
                       "frame %d/%d)\n", i + 1, stack_pages);
            kfree(kstack);
            free_user_address_space(address_space);
            return -1;
        }
        uint32_t virt = USER_STACK_VIRT_BASE + (uint32_t)i * 4096u;
        paging_map_page(pd, virt, frame,
                         PAGE_PRESENT | PAGE_WRITE | PAGE_USER);
    }
    uint32_t ustack_top = USER_STACK_VIRT_BASE + USER_STACK_SIZE;

    /* Builds the real x86 process-entry stack convention (the same
     * raw layout the Linux kernel's own execve() leaves for a fresh
     * process, which a real C runtime's _start then treats argv/envp
     * as pointers into): from the initial ESP, low to high addresses:
     * argc, argv[0..argc-1] (each a pointer into the string data
     * below), a NULL terminator, envp[0..envc-1] (Phase 73: the
     * environment strings the caller passed - previously always empty),
     * a second NULL terminator, then the argv and environment strings
     * themselves. Built top-down since the string data's addresses
     * need to be known before the pointer arrays referencing them can
     * be written. */
    uint32_t write_ptr = ustack_top;
    uint32_t argv_addrs[MAX_EXEC_ARGS];
    uint32_t env_addrs[MAX_EXEC_ENV];

    for (int i = 0; i < argc; i++) {
        uint32_t len = (uint32_t)strlen(argv[i]) + 1;
        write_ptr -= len;
        write_to_address_space(pd, write_ptr, argv[i], len);
        argv_addrs[i] = write_ptr;
    }

    for (int i = 0; i < envc; i++) {
        uint32_t len = (uint32_t)strlen(envp[i]) + 1;
        write_ptr -= len;
        write_to_address_space(pd, write_ptr, envp[i], len);
        env_addrs[i] = write_ptr;
    }

    write_ptr &= ~0x3u; /* 4-byte align before the pointer arrays */

    uint32_t zero = 0;
    write_ptr -= 4;
    write_to_address_space(pd, write_ptr, &zero, 4); /* envp terminator */
    for (int i = envc - 1; i >= 0; i--) {
        write_ptr -= 4;
        write_to_address_space(pd, write_ptr, &env_addrs[i], 4);
    }
    write_ptr -= 4;
    write_to_address_space(pd, write_ptr, &zero, 4); /* argv terminator */

    for (int i = argc - 1; i >= 0; i--) {
        write_ptr -= 4;
        write_to_address_space(pd, write_ptr, &argv_addrs[i], 4);
    }

    write_ptr -= 4;
    write_to_address_space(pd, write_ptr, &argc, 4);

    uint32_t initial_esp = write_ptr;

    uint32_t kstack_top = (uint32_t)kstack + KERNEL_STACK_SIZE;
    uint32_t* sp = (uint32_t*)kstack_top;

    /* Same fake IRET frame construction as create_user_task_common(),
     * just pointing EIP at the ELF's own entry point and ESP at the
     * constructed argv/argc stack area instead of a fixed kernel
     * function and a bare stack top. */
    *(--sp) = GDT_USER_DATA;
    *(--sp) = initial_esp;
    *(--sp) = 0x202;
    *(--sp) = GDT_USER_CODE;
    *(--sp) = entry_point;

    *(--sp) = (uint32_t)enter_usermode;
    *(--sp) = 0;
    *(--sp) = 0;
    *(--sp) = 0;
    *(--sp) = 0;
    *(--sp) = 0x202;

    /* pid already assigned atomically by allocate_slot() itself. */
    copy_name(p->name, path, sizeof(p->name));
    p->is_user = true;
    p->esp = (uint32_t)sp;
    p->kernel_stack_top = kstack_top;
    p->kernel_stack_alloc = kstack;
    p->page_directory_phys = address_space;
    p->allowed_file_count = 0;
    if (filenames != NULL && file_count > 0) {
        if (file_count > MAX_CAPABILITIES) {
            file_count = MAX_CAPABILITIES;
        }
        for (int i = 0; i < file_count; i++) {
            copy_name(p->allowed_files[i], filenames[i],
                      sizeof(p->allowed_files[i]));
        }
        p->allowed_file_count = file_count;
    }
    p->can_open_any_file = grant_any_file;
    p->allowed_host_count = 0;
    /* Phase 71: grant_spawn is its own, independent parameter now -
     * see this function's own doc comment above for why. A trusted,
     * general-purpose shell (process_exec_as_shell(), which still
     * passes true for both) needs both broad file access and the
     * ability to run programs, and for that one, fully-trusted case
     * they really are the same underlying trust decision - but that
     * is no longer the only real, legitimate shape a caller's own
     * grants can take. */
    p->can_spawn = grant_spawn;
    p->exit_code = 0;
    p->heap_current = HEAP_VIRT_BASE;
    p->heap_mapped_end = HEAP_VIRT_BASE;

    /* Real exec() semantics, deliberately different from this
     * function's own capability handling just above (allowed_files[]/
     * can_open_any_file/can_spawn all reset to nothing-or-an-explicit-
     * grant on every exec, by original design) - uid/gid represent
     * *who is running this*, which real exec() does not change, so
     * this inherits the calling process's identity rather than
     * resetting it. process_current() is NULL only for the very first
     * exec at boot (kernel_main() loading the initial shell, before
     * the scheduler has started running anything) - that one case
     * starts as root (uid 0), matching Unix's own PID 1/init
     * convention. */
    process_t* caller = process_current();
    p->uid = (caller != NULL) ? caller->uid : 0;
    p->gid = (caller != NULL) ? caller->gid : 0;

    kernel_log("[ OK ] process_exec: loaded '%s' as pid %d, entry=0x%x, "
               "%d arg(s)\n", path, p->pid, entry_point, argc);

    process_publish(p);
    return p->pid;
}

int process_exec(const char* path, const char** argv, int argc) {
    return process_exec_internal(path, argv, argc, NULL, 0, false, false,
                                  NULL, 0);
}

/* Phase 73: see process.h. Grants exactly what plain process_exec()
 * grants - nothing - and differs only in the environment it passes. */
int process_exec_env(const char* path, const char** argv, int argc,
                     const char** envp, int envc) {
    return process_exec_internal(path, argv, argc, NULL, 0, false, false,
                                  envp, envc);
}

/* Phase 29: for trusted (ring-0) callers only - lets a caller grant
 * specific file capabilities to the process being created, the same
 * least-privilege pattern process_create_sandboxed_task() (Phase 11)
 * already established, applied to exec'd processes rather than only
 * kernel-compiled demo tasks. The ordinary ring-3 SYS_EXEC syscall
 * still only ever reaches plain process_exec() above (granting
 * nothing) - this exists so a future, more capable shell (or, for
 * now, a boot self-test proving a real ring-3 coreutils program
 * works correctly once actually given the access it needs) can
 * deliberately choose what a program it launches may open, rather
 * than either the shell needing its own privilege to grant arbitrary
 * access at runtime (a much bigger, separate design question) or
 * every exec'd program needing blanket file access by default (which
 * would quietly weaken Phase 11's least-privilege model instead of
 * extending it). */
int process_exec_with_files(const char* path, const char** argv, int argc,
                             const char** filenames, int file_count) {
    return process_exec_internal(path, argv, argc, filenames, file_count,
                                  false, false, NULL, 0);
}

/* Phase 30: exec's a process with broad, "may open any file" access
 * (see can_open_any_file's comment in process.h) - reserved for the
 * one genuinely trusted, general-purpose program that needs it: the
 * interactive shell itself, which has to open whatever file the user
 * names at a prompt, not a small set known in advance. Not exposed to
 * ring-3 SYS_EXEC, and not something an ordinary exec'd program (like
 * userland/coreutils/cat.c) receives even indirectly - only the
 * kernel's own boot sequence calls this, for the shell specifically. */
int process_exec_as_shell(const char* path, const char** argv, int argc) {
    return process_exec_internal(path, argv, argc, NULL, 0, true, true,
                                  NULL, 0);
}

/* Phase 59: SYS_EXEC_TRUSTED's own implementation - see process.h's
 * own comment on this function, and kernel/arch/x86/cpu/syscall.h's
 * comment on SYS_EXEC_TRUSTED, for the full reasoning. Unlike every
 * process_exec_*() variant above, the grants passed to process_exec_
 * internal() here are not fixed constants (false, or true only for
 * the one kernel-boot-time shell exec) - each is read from the
 * calling process itself, so this function's own behavior is entirely
 * determined by who calls it: an ordinary process delegates "false"
 * for both (a no-op, identical to plain process_exec()), while a
 * process that already has can_open_any_file and/or can_spawn
 * delegates that same real grant to the child it's choosing to run -
 * independently of each other (Phase 71: see process_exec_internal()'s
 * own doc comment on why grant_spawn is a separate parameter now, not
 * folded into grant_any_file - a caller can genuinely have can_spawn
 * without can_open_any_file, such as a sandboxed task explicitly
 * granted spawn but nothing broader, and this must still be able to
 * delegate the one real capability it actually has). */
int process_exec_trusted_env(const char* path, const char** argv, int argc,
                             const char** envp, int envc) {
    process_t* caller = process_current();
    bool grant_files = (caller != NULL) && caller->can_open_any_file;
    bool grant_spawn = (caller != NULL) && caller->can_spawn;
    return process_exec_internal(path, argv, argc, NULL, 0, grant_files,
                                  grant_spawn, envp, envc);
}

/* The original entry point: identical grants, empty environment. */
int process_exec_trusted(const char* path, const char** argv, int argc) {
    return process_exec_trusted_env(path, argv, argc, NULL, 0);
}

/* Phase 71: the actual "check once, reap if terminated" logic shared
 * between process_wait() (below, unchanged in observable behavior -
 * loops calling this until it succeeds) and process_wait_nonblock()
 * (kernel/task/process.h - Phase 71's own service supervisor needs to
 * check on several children without blocking on any single one, which
 * process_wait()'s own infinite loop can't do). Every safety argument
 * in the two, real, previously-fixed use-after-free bugs this
 * function's own body still carries in full (the kernel-stack free
 * and the page-directory free, both explained in detail just below)
 * applies identically regardless of which of the two callers reaches
 * this point - neither one ever runs on `target`'s own stack or with
 * `target`'s own page directory loaded, whether they got here by
 * blocking-and-retrying or by a single, immediate check. Returns
 * `true` (with `*out_exit_code` set) if `pid` was found and had
 * already reached PROCESS_TERMINATED - reaping it, exactly once, the
 * same as process_wait() always has; `false` (leaving
 * `*out_exit_code` untouched) if `pid` doesn't exist at all, or
 * exists but hasn't terminated yet - the caller's own job to tell
 * those two, genuinely different cases apart if it cares to (process_
 * wait_nonblock()'s own doc comment shows how). */
static bool try_reap_process(int pid, int* out_exit_code) {
    process_t* target = NULL;
    for (int i = 0; i < MAX_PROCESSES; i++) {
        process_t* candidate = &process_table[i];
        if (candidate->pid == pid && candidate->state != PROCESS_UNUSED) {
            target = candidate;
            break;
        }
    }
    if (target == NULL) {
        return false; /* no such process - never existed, or already
                          reaped and its slot is unused again (slots
                          are never actually recycled today - see
                          PROGRESS.md - but this check is the correct
                          behavior regardless) */
    }
    if (target->state != PROCESS_TERMINATED) {
        return false;
    }

    /* The actual kernel-stack free lives here now, not in
     * process_exit_current() - see that function's own comment for
     * the full account of the real, confirmed use-after-free this
     * replaces. Safe specifically because: this code runs on this
     * function's own caller's stack, never on `target`'s; and by the
     * time state is observed as PROCESS_TERMINATED, `target` has
     * already reached its own scheduler_yield() call and - since a
     * TERMINATED process is never returned by pick_next_locked() -
     * can never be scheduled again, so nothing will ever execute on
     * its kernel stack after this point. Guarded with the same
     * null-check-then-null-out pattern the original, unsafe version
     * already used, since this function - and process_wait() itself,
     * before this refactor, and process_wait_nonblock() now too - can
     * genuinely be called more than once for the same pid (this
     * project's own sandbox_demo.c does exactly that in several of
     * its own tests) and this must stay safe to call repeatedly, not
     * just once. */
    if (target->kernel_stack_alloc != NULL) {
        kfree(target->kernel_stack_alloc);
        target->kernel_stack_alloc = NULL;
    }
    /* A second, more severe instance of the exact same class of bug
     * the kernel-stack free above already fixed - found on further
     * investigation after that first fix reduced but did not
     * eliminate a still-observed, non-deterministic corruption.
     * free_user_address_space() used to be called directly from
     * process_exit_current(), which - for a user process - means it
     * ran while that process's own page directory was still the
     * CPU's *actively loaded* CR3 value. That function's own last
     * step is pmm_free_frame(page_directory_phys) - handing the
     * exact physical frame the CPU is using *right now* to translate
     * every single memory access (including the rest of process_exit_
     * current() and scheduler_yield() finishing their own execution)
     * back to the PMM's free list, where any other pmm_alloc_frame()
     * call anywhere in the kernel could immediately claim and
     * overwrite it - silently corrupting the live page directory out
     * from under the still-running CPU. This explains the wide,
     * seemingly unrelated variety of symptoms better than the
     * kernel-stack bug alone did: a corrupted page directory produces
     * unpredictable translation failures for whatever gets accessed
     * next, not a single consistent failure mode. Moved here for the
     * identical reason and with the identical safety argument as the
     * kernel-stack free just above - by this point `target` is
     * guaranteed to have already switched away (a TERMINATED
     * process's own page directory is never reloaded into CR3 again,
     * since do_schedule() only loads `next`'s), and this code runs on
     * the *caller's* own, different, already-active page directory,
     * not `target`'s. page_directory_phys is set to 0 after freeing
     * (0 is never a valid page directory physical address) as this
     * function's own guard against a second reap attempt on the same
     * pid trying to free it again. */
    if (target->is_user && target->page_directory_phys != 0) {
        free_user_address_space(target->page_directory_phys);
        target->page_directory_phys = 0;
    }
    *out_exit_code = target->exit_code;
    return true;
}

int process_wait(int pid) {
    for (;;) {
        int exit_code;
        bool exists;
        if (process_wait_nonblock(pid, &exit_code, &exists)) {
            return exit_code;
        }
        if (!exists) {
            return -1; /* no such process - never existed, or already
                           reaped and its slot is unused again (slots
                           are never actually recycled today - see
                           PROGRESS.md - but this check is the correct
                           behavior regardless). Preserved exactly from
                           this function's own pre-Phase-71 behavior -
                           a real regression this refactor introduced
                           once already (looping forever here instead,
                           since try_reap_process() alone can't tell
                           "doesn't exist" apart from "not terminated
                           yet") and then caught and fixed before ever
                           shipping it, not after. */
        }
        scheduler_yield();
    }
}

bool process_wait_nonblock(int pid, int* out_exit_code, bool* out_exists) {
    bool exists = false;
    for (int i = 0; i < MAX_PROCESSES; i++) {
        if (process_table[i].pid == pid &&
            process_table[i].state != PROCESS_UNUSED) {
            exists = true;
            break;
        }
    }
    if (out_exists != NULL) {
        *out_exists = exists;
    }
    if (!exists) {
        return false;
    }
    return try_reap_process(pid, out_exit_code);
}

uint32_t process_sbrk(process_t* p, int increment) {
    if (p == NULL || increment < 0) {
        return (uint32_t)-1;
    }

    uint32_t old_break = p->heap_current;
    uint32_t new_break = old_break + (uint32_t)increment;
    uint32_t* pd = (uint32_t*)p->page_directory_phys;

    while (p->heap_mapped_end < new_break) {
        uint32_t frame = pmm_alloc_frame();
        if (frame == 0) {
            return (uint32_t)-1; /* out of physical memory - the heap
                                     stays at whatever it grew to
                                     before this call, matching real
                                     sbrk()'s all-or-nothing failure */
        }
        paging_map_page(pd, p->heap_mapped_end, frame,
                         PAGE_PRESENT | PAGE_WRITE | PAGE_USER);
        p->heap_mapped_end += 4096;
    }

    p->heap_current = new_break;
    return old_break;
}

int process_fork(registers_t* parent_regs) {
    process_t* parent = scheduler_current();
    if (parent == NULL) {
        return -1;
    }

    process_t* child = allocate_slot();
    if (child == NULL) {
        return -1;
    }

    void* child_kstack = kmalloc(KERNEL_STACK_SIZE);
    if (child_kstack == NULL) {
        return -1;
    }

    uint32_t child_pd_phys = paging_create_address_space();
    if (!cow_share_address_space(parent->page_directory_phys,
                                  child_pd_phys)) {
        kfree(child_kstack);
        free_user_address_space(child_pd_phys);
        return -1;
    }

    /* Build the child's kernel stack: a full copy of the parent's
     * saved register state (registers_t - see kernel/arch/x86/cpu/
     * isr.h) at the moment of this syscall, with eax overwritten to 0
     * (fork()'s child-side return value), followed by the fake
     * switch_context() frame that makes scheduling this process in
     * for the first time land at syscall_return_point instead of
     * enter_usermode - see that label's comment in syscall_stub.asm
     * for the full picture, and create_user_task_common() above for
     * the same "push in reverse field order, starting from the top of
     * a fresh kernel stack" technique this reuses. */
    uint32_t kstack_top = (uint32_t)child_kstack + KERNEL_STACK_SIZE;
    uint32_t* sp = (uint32_t*)kstack_top;

    *(--sp) = parent_regs->ss;
    *(--sp) = parent_regs->useresp;
    *(--sp) = parent_regs->eflags;
    *(--sp) = parent_regs->cs;
    *(--sp) = parent_regs->eip;
    *(--sp) = parent_regs->err_code;
    *(--sp) = parent_regs->int_no;
    *(--sp) = 0; /* eax: the child's fork() return value is always 0 */
    *(--sp) = parent_regs->ecx;
    *(--sp) = parent_regs->edx;
    *(--sp) = parent_regs->ebx;
    *(--sp) = parent_regs->esp_dummy; /* POPA discards this slot rather
                                          than actually loading ESP
                                          from it - the value here is
                                          never used, copied only for
                                          structural completeness */
    *(--sp) = parent_regs->ebp;
    *(--sp) = parent_regs->esi;
    *(--sp) = parent_regs->edi;
    *(--sp) = parent_regs->ds;

    *(--sp) = (uint32_t)syscall_return_point;
    *(--sp) = 0; /* ebp */
    *(--sp) = 0; /* ebx */
    *(--sp) = 0; /* esi */
    *(--sp) = 0; /* edi */
    *(--sp) = 0x202; /* eflags */

    /* pid already assigned atomically by allocate_slot() itself. */
    copy_name(child->name, parent->name, sizeof(child->name));
    child->is_user = true;
    child->esp = (uint32_t)sp;
    child->kernel_stack_top = kstack_top;
    child->kernel_stack_alloc = child_kstack;
    child->page_directory_phys = child_pd_phys;
    child->exit_code = 0;

    /* Real fork() semantics: the child inherits the parent's
     * capabilities and heap state (the heap's actual memory is itself
     * now COW-shared, so both processes see identical contents until
     * either one writes to it) - unlike process_exec() (Phase 23),
     * which deliberately starts a new process with none of the
     * caller's privileges. */
    child->allowed_file_count = parent->allowed_file_count;
    memcpy(child->allowed_files, parent->allowed_files,
           sizeof(parent->allowed_files));
    child->allowed_host_count = parent->allowed_host_count;
    memcpy(child->allowed_hosts, parent->allowed_hosts,
           sizeof(parent->allowed_hosts));
    child->can_spawn = parent->can_spawn;
    child->can_open_any_file = parent->can_open_any_file;
    child->heap_current = parent->heap_current;
    child->heap_mapped_end = parent->heap_mapped_end;
    child->uid = parent->uid; /* real fork() semantics - a child is
                                  still "the same user" as its parent,
                                  same reasoning as process_exec_internal()'s
                                  own inheritance, see process.h's own
                                  comment on process_t's uid/gid */
    child->gid = parent->gid;

    kernel_log("[ OK ] process_fork: pid %d forked -> new pid %d\n",
               parent->pid, child->pid);

    process_publish(child);
    return child->pid;
}

void process_set_identity(process_t* p, uint32_t uid, uint32_t gid) {
    p->uid = uid;
    p->gid = gid;
}

/* kernel/rust/users.rs's exported authentication check - see that
 * file's own doc comment for the full contract, including why its
 * password hash is explicitly not a secure one. */
extern bool rust_users_authenticate(const uint8_t* username_ptr,
                                     uint32_t username_len,
                                     const uint8_t* password_ptr,
                                     uint32_t password_len, uint32_t* out_uid,
                                     uint32_t* out_gid);

bool process_login(process_t* p, const char* username, const char* password) {
    uint32_t uid = 0;
    uint32_t gid = 0;
    bool ok = rust_users_authenticate(
        (const uint8_t*)username, (uint32_t)strlen(username),
        (const uint8_t*)password, (uint32_t)strlen(password), &uid, &gid);
    if (!ok) {
        return false;
    }
    process_set_identity(p, uid, gid);
    return true;
}

/* kernel/rust/users.rs's exported sudo gate - see that file's own doc
 * comment for the full contract. */
extern bool rust_users_sudo_check(uint32_t uid, const uint8_t* password_ptr,
                                   uint32_t password_len);

bool process_sudo(process_t* p, const char* password) {
    bool ok = rust_users_sudo_check((uint32_t)p->uid, (const uint8_t*)password,
                                     (uint32_t)strlen(password));
    if (!ok) {
        return false;
    }
    process_set_identity(p, 0, 0);
    return true;
}
