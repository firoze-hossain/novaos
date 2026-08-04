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
#include "../arch/x86/cpu/gdt.h"
#include "../arch/x86/mm/heap.h"
#include "../arch/x86/mm/paging.h"
#include "../arch/x86/mm/pmm.h"
#include "../lib/string.h"
#include "../include/kernel.h"

extern void enter_usermode(void);

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
static void free_user_address_space(uint32_t page_directory_phys) {
    uint32_t* pd = (uint32_t*)page_directory_phys;
    uint32_t pd_index = USER_STACK_VIRT_BASE >> 22;

    if (pd[pd_index] & PAGE_PRESENT) {
        uint32_t pt_phys = pd[pd_index] & 0xFFFFF000u;
        uint32_t* pt = (uint32_t*)pt_phys;
        uint32_t first_pt_entry = (USER_STACK_VIRT_BASE >> 12) & 0x3FFu;
        uint32_t stack_pages = USER_STACK_SIZE / 4096;

        for (uint32_t i = 0; i < stack_pages; i++) {
            uint32_t entry = pt[first_pt_entry + i];
            if (entry & PAGE_PRESENT) {
                pmm_free_frame(entry & 0xFFFFF000u);
            }
        }
        pmm_free_frame(pt_phys);
    }

    pmm_free_frame(page_directory_phys);
}

void process_exit_current(void) {
    process_t* p = scheduler_current();
    if (p != NULL) {
        p->state = PROCESS_TERMINATED;
        kernel_log("[ OK ] Process '%s' (pid %d) exited\n", p->name, p->pid);

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
