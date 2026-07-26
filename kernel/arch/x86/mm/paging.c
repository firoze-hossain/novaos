/*
 * paging.c - x86 32-bit paging (identity-mapped boot configuration)
 *
 * Sets up a two-level page directory/page table structure identity-
 * mapping the first 64MB of physical memory 1:1 to the same virtual
 * addresses, then flips CR0.PG on. Nothing in the kernel's addressing
 * changes as a result (it was already linked to run at 0x100000 and
 * everything it touches is below 64MB), but this is the prerequisite
 * for anything Phase 4+ wants to do with real virtual memory (per-
 * process address spaces, guard pages, copy-on-write, etc.) - the
 * kernel now runs *through* the MMU instead of the MMU being off.
 *
 * Known limitation (see PROGRESS.md): this is 32-bit non-PAE paging,
 * so page table entries have no NX (non-executable) bit - that
 * requires PAE or long mode. Every mapped page here is both writable
 * and executable. Real W^X enforcement is tracked as future work.
 */
#include "paging.h"
#include "../cpu/isr.h"
#include "../cpu/idt.h"
#include "../../../include/kernel.h"

#define PAGE_PRESENT 0x1
#define PAGE_WRITE   0x2
#define PAGE_USER    0x4

#define IDENTITY_MAP_TABLES 16 /* 16 * 4MB = 64MB */

static uint32_t page_directory[1024] __attribute__((aligned(4096)));
static uint32_t page_tables[IDENTITY_MAP_TABLES][1024]
    __attribute__((aligned(4096)));

static inline uint32_t read_cr2(void) {
    uint32_t val;
    __asm__ volatile ("mov %%cr2, %0" : "=r"(val));
    return val;
}

static void page_fault_handler(registers_t* regs) {
    uint32_t faulting_address = read_cr2();

    /* Intel SDM Vol 3A, 4.7: page-fault error code bit layout. */
    bool present  = regs->err_code & 0x1;
    bool write    = regs->err_code & 0x2;
    bool user     = regs->err_code & 0x4;
    bool reserved = regs->err_code & 0x8;

    kernel_log("[FAULT] Page fault at 0x%x (eip=0x%x): %s, %s, %s%s\n",
               (int)faulting_address, (int)regs->eip,
               present ? "protection violation" : "page not present",
               write ? "write" : "read",
               user ? "user mode" : "kernel mode",
               reserved ? ", reserved bit set" : "");

    kernel_panic("Page Fault");
}

static void build_identity_map(void) {
    /* PAGE_USER on both levels: x86 ANDs the U/S bit across the page
     * directory entry and the page table entry, so both need it for
     * ring-3 code to access a page at all. This does mean every page
     * in the identity map - not just the Phase 4 demo task's own code
     * and stack - is technically ring-3-readable/writable, which
     * sounds like it undoes the ring 0/3 boundary. It doesn't undo
     * anything that existed: there is still no per-process address
     * space (see PROGRESS.md, tracked since Phase 3), so there was
     * never page-level kernel/user memory separation to begin with -
     * only the CPL check on privileged instructions was real
     * enforcement until now. Real memory isolation needs per-process
     * page directories, which is Phase 5+ scope. */
    for (int t = 0; t < IDENTITY_MAP_TABLES; t++) {
        for (int e = 0; e < 1024; e++) {
            uint32_t phys = (uint32_t)(t * 1024 + e) * 4096u;
            page_tables[t][e] = phys | PAGE_PRESENT | PAGE_WRITE | PAGE_USER;
        }
        page_directory[t] = ((uint32_t)&page_tables[t][0]) | PAGE_PRESENT |
                             PAGE_WRITE | PAGE_USER;
    }

    for (int d = IDENTITY_MAP_TABLES; d < 1024; d++) {
        page_directory[d] = 0; /* not present */
    }
}

void paging_init(void) {
    register_exception_handler(14, page_fault_handler);

    build_identity_map();

    __asm__ volatile (
        "mov %0, %%cr3\n"
        "mov %%cr0, %%eax\n"
        "or $0x80000000, %%eax\n"
        "mov %%eax, %%cr0\n"
        :
        : "r"(page_directory)
        : "eax"
    );

    kernel_log("[ OK ] Paging enabled (identity-mapped 0-64MB)\n");
}
