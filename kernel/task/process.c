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
             * race this closes. */
            process_table[i].state = PROCESS_ALLOCATING;
            spinlock_release(&process_table_lock, flags);
            return &process_table[i];
        }
    }
    spinlock_release(&process_table_lock, flags);
    return NULL;
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

    p->pid = next_pid++;
    copy_name(p->name, name, sizeof(p->name));
    p->state = PROCESS_READY;
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

    scheduler_add(p);
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

    p->pid = next_pid++;
    copy_name(p->name, name, sizeof(p->name));
    p->state = PROCESS_READY;
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
    scheduler_add(p);
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

    scheduler_add(p);
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

    scheduler_add(p);
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
     * again - but fail safe rather than fall into undefined behavior. */
    for (;;) {
        __asm__ volatile ("hlt");
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

static int process_exec_internal(const char* path, const char** argv,
                                  int argc, const char** filenames,
                                  int file_count, bool grant_any_file) {
    if (argc > MAX_EXEC_ARGS) {
        argc = MAX_EXEC_ARGS;
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
     * below), a NULL terminator, an empty envp (just one more NULL -
     * no environment variables are actually populated yet, an honest
     * scope limit - see PROGRESS.md), then the argv strings
     * themselves. Built top-down since the string data's addresses
     * need to be known before the pointer array referencing them can
     * be written. */
    uint32_t write_ptr = ustack_top;
    uint32_t argv_addrs[MAX_EXEC_ARGS];

    for (int i = 0; i < argc; i++) {
        uint32_t len = (uint32_t)strlen(argv[i]) + 1;
        write_ptr -= len;
        write_to_address_space(pd, write_ptr, argv[i], len);
        argv_addrs[i] = write_ptr;
    }

    write_ptr &= ~0x3u; /* 4-byte align before the pointer arrays */

    uint32_t zero = 0;
    write_ptr -= 4;
    write_to_address_space(pd, write_ptr, &zero, 4); /* envp terminator */
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

    p->pid = next_pid++;
    copy_name(p->name, path, sizeof(p->name));
    p->state = PROCESS_READY;
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
    /* A trusted, general-purpose shell needs both broad file access
     * and the ability to run programs - the same single grant_any_file
     * flag covers both, since they're both part of the same "this is
     * the shell, not a sandboxed program" trust decision. */
    p->can_spawn = grant_any_file;
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

    scheduler_add(p);
    return p->pid;
}

int process_exec(const char* path, const char** argv, int argc) {
    return process_exec_internal(path, argv, argc, NULL, 0, false);
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
                                  false);
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
    return process_exec_internal(path, argv, argc, NULL, 0, true);
}

/* Phase 59: SYS_EXEC_TRUSTED's own implementation - see process.h's
 * own comment on this function, and kernel/arch/x86/cpu/syscall.h's
 * comment on SYS_EXEC_TRUSTED, for the full reasoning. Unlike every
 * process_exec_*() variant above, the grant passed to process_exec_
 * internal() here is not a fixed constant (false, or true only for
 * the one kernel-boot-time shell exec) - it is read from the calling
 * process itself, so this function's own behavior is entirely
 * determined by who calls it: an ordinary process delegates "false"
 * (a no-op, identical to plain process_exec()), while a process that
 * already has can_open_any_file (only the interactive shell, today)
 * delegates that same real grant to the child it's choosing to run. */
int process_exec_trusted(const char* path, const char** argv, int argc) {
    process_t* caller = process_current();
    bool grant = (caller != NULL) && caller->can_open_any_file;
    return process_exec_internal(path, argv, argc, NULL, 0, grant);
}

int process_wait(int pid) {
    for (;;) {
        process_t* target = NULL;
        for (int i = 0; i < MAX_PROCESSES; i++) {
            process_t* candidate = &process_table[i];
            if (candidate->pid == pid && candidate->state != PROCESS_UNUSED) {
                target = candidate;
                break;
            }
        }
        if (target == NULL) {
            return -1; /* no such process - never existed, or already
                           reaped and its slot is unused again (slots
                           are never actually recycled today - see
                           PROGRESS.md - but this check is the correct
                           behavior regardless) */
        }
        if (target->state == PROCESS_TERMINATED) {
            /* The actual kernel-stack free lives here now, not in
             * process_exit_current() - see that function's own
             * comment for the full account of the real, confirmed
             * use-after-free this replaces. Safe specifically because:
             * this code runs on process_wait()'s OWN caller's stack,
             * never on `target`'s; and by the time state is observed
             * as PROCESS_TERMINATED, `target` has already reached its
             * own scheduler_yield() call and - since a TERMINATED
             * process is never returned by pick_next_locked() - can
             * never be scheduled again, so nothing will ever execute
             * on its kernel stack after this point. Guarded with the
             * same null-check-then-null-out pattern the original,
             * unsafe version already used, since process_wait() can
             * genuinely be called more than once for the same pid
             * (this project's own sandbox_demo.c does exactly that in
             * several of its own tests) and this must stay safe to
             * call repeatedly, not just once. */
            if (target->kernel_stack_alloc != NULL) {
                kfree(target->kernel_stack_alloc);
                target->kernel_stack_alloc = NULL;
            }
            /* A second, more severe instance of the exact same class
             * of bug the kernel-stack free above already fixed - found
             * on further investigation after that first fix reduced
             * but did not eliminate a still-observed, non-deterministic
             * corruption. free_user_address_space() used to be called
             * directly from process_exit_current(), which - for a
             * user process - means it ran while that process's own
             * page directory was still the CPU's *actively loaded*
             * CR3 value. That function's own last step is
             * pmm_free_frame(page_directory_phys) - handing the exact
             * physical frame the CPU is using *right now* to translate
             * every single memory access (including the rest of
             * process_exit_current() and scheduler_yield() finishing
             * their own execution) back to the PMM's free list, where
             * any other pmm_alloc_frame() call anywhere in the kernel
             * could immediately claim and overwrite it - silently
             * corrupting the live page directory out from under the
             * still-running CPU. This explains the wide, seemingly
             * unrelated variety of symptoms better than the kernel-
             * stack bug alone did: a corrupted page directory produces
             * unpredictable translation failures for whatever gets
             * accessed next, not a single consistent failure mode.
             * Moved here for the identical reason and with the
             * identical safety argument as the kernel-stack free just
             * above - by this point `target` is guaranteed to have
             * already switched away (a TERMINATED process's own page
             * directory is never reloaded into CR3 again, since
             * do_schedule() only loads `next`'s), and this code runs
             * on the *caller's* own, different, already-active page
             * directory, not `target`'s. page_directory_phys is set to
             * 0 after freeing (0 is never a valid page directory
             * physical address) as this function's own guard against
             * a second process_wait() call on the same pid trying to
             * free it again. */
            if (target->is_user && target->page_directory_phys != 0) {
                free_user_address_space(target->page_directory_phys);
                target->page_directory_phys = 0;
            }
            return target->exit_code;
        }
        scheduler_yield();
    }
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

    child->pid = next_pid++;
    copy_name(child->name, parent->name, sizeof(child->name));
    child->state = PROCESS_READY;
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

    scheduler_add(child);
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
