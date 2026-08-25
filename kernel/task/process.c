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
}

static process_t* allocate_slot(void) {
    for (int i = 0; i < MAX_PROCESSES; i++) {
        if (process_table[i].state == PROCESS_UNUSED) {
            return &process_table[i];
        }
    }
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
    p->exit_code = 0;
    p->heap_current = HEAP_VIRT_BASE;
    p->heap_mapped_end = HEAP_VIRT_BASE;

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
    p->exit_code = 0;
    p->heap_current = HEAP_VIRT_BASE;
    p->heap_mapped_end = HEAP_VIRT_BASE;

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
            if (pt[j] & PAGE_PRESENT) {
                pmm_free_frame(pt[j] & 0xFFFFF000u);
            }
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

        if (p->kernel_stack_alloc != NULL) {
            kfree(p->kernel_stack_alloc);
            p->kernel_stack_alloc = NULL;
        }
        if (p->is_user) {
            free_user_address_space(p->page_directory_phys);
        }
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

int process_exec(const char* path, const char** argv, int argc) {
    if (argc > MAX_EXEC_ARGS) {
        argc = MAX_EXEC_ARGS;
    }

    /* 64KB comfortably covers the small, statically-linked test
     * binaries this loader targets; a much larger real-world program
     * would need this read streamed rather than buffered whole, which
     * vfs_read_file() doesn't support yet - see PROGRESS.md. */
    static uint8_t elf_buffer[65536];
    int file_size = vfs_read_file(path, elf_buffer, sizeof(elf_buffer));
    if (file_size <= 0) {
        kernel_log("[FAULT] process_exec: couldn't read '%s'\n", path);
        return -1;
    }
    if (!elf_validate(elf_buffer, (uint32_t)file_size)) {
        kernel_log("[FAULT] process_exec: '%s' is not a valid ELF32 "
                   "executable this loader supports\n", path);
        return -1;
    }

    process_t* p = allocate_slot();
    if (p == NULL) {
        kernel_log("[FAULT] process_exec: process table full\n");
        return -1;
    }

    void* kstack = kmalloc(KERNEL_STACK_SIZE);
    if (kstack == NULL) {
        kernel_log("[FAULT] process_exec: out of memory (kstack)\n");
        return -1;
    }

    uint32_t address_space = paging_create_address_space();
    uint32_t* pd = (uint32_t*)address_space;

    uint32_t entry_point;
    if (!elf_load(elf_buffer, (uint32_t)file_size, address_space,
                   &entry_point)) {
        kernel_log("[FAULT] process_exec: failed to load '%s'\n", path);
        kfree(kstack);
        free_user_address_space(address_space);
        return -1;
    }

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
    p->allowed_host_count = 0;
    p->can_spawn = false;
    p->exit_code = 0;
    p->heap_current = HEAP_VIRT_BASE;
    p->heap_mapped_end = HEAP_VIRT_BASE;

    kernel_log("[ OK ] process_exec: loaded '%s' as pid %d, entry=0x%x, "
               "%d arg(s)\n", path, p->pid, entry_point, argc);

    scheduler_add(p);
    return p->pid;
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
    child->heap_current = parent->heap_current;
    child->heap_mapped_end = parent->heap_mapped_end;

    kernel_log("[ OK ] process_fork: pid %d forked -> new pid %d\n",
               parent->pid, child->pid);

    scheduler_add(child);
    return child->pid;
}
