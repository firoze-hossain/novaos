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
#include "pmm.h"
#include "../cpu/isr.h"
#include "../cpu/idt.h"
#include "../../../lib/string.h"
#include "../../../include/kernel.h"

#define IDENTITY_MAP_TABLES 16 /* 16 * 4MB = 64MB */

static uint32_t page_directory[1024] __attribute__((aligned(4096)));
static uint32_t page_tables[IDENTITY_MAP_TABLES][1024]
    __attribute__((aligned(4096)));

static inline uint32_t read_cr2(void) {
    uint32_t val;
    __asm__ volatile ("mov %%cr2, %0" : "=r"(val));
    return val;
}

/* Phase 27: resolves a write fault to a copy-on-write page (see
 * PAGE_COW's comment in paging.h) by giving the faulting process its
 * own private copy, instead of it being a real error. Reads CR3
 * directly rather than going through process_current() specifically
 * to avoid paging.c taking on a new dependency on process.h for
 * something this self-contained - the currently-loaded page directory
 * is exactly the faulting process's own, since a page fault (like any
 * exception) doesn't switch CR3 the same way a syscall doesn't (see
 * Phase 11's PROGRESS.md note on that same property). Returns false
 * for anything that isn't actually a resolvable COW fault, so the
 * caller falls through to the ordinary page-fault panic. */
static bool try_resolve_cow_fault(uint32_t faulting_address) {
    uint32_t cr3;
    __asm__ volatile ("mov %%cr3, %0" : "=r"(cr3));
    uint32_t* pd = (uint32_t*)cr3;

    uint32_t pd_index = faulting_address >> 22;
    uint32_t pt_index = (faulting_address >> 12) & 0x3FFu;

    if (!(pd[pd_index] & PAGE_PRESENT)) {
        return false;
    }
    uint32_t* pt = (uint32_t*)(pd[pd_index] & 0xFFFFF000u);
    uint32_t entry = pt[pt_index];

    if (!(entry & PAGE_PRESENT) || !(entry & PAGE_COW)) {
        return false; /* not present at all, or present but not a COW
                          page - a genuine protection violation either
                          way, not this function's job to resolve */
    }

    uint32_t old_frame = entry & 0xFFFFF000u;
    uint32_t new_frame = pmm_alloc_frame();
    if (new_frame == 0) {
        return false; /* out of memory - let the panic below report it */
    }

    /* Both frames are reachable directly through this kernel's
     * identity-mapped low memory (virtual address == physical address
     * there) - the same reasoning every DMA buffer and ELF loader
     * segment in this tree already relies on. */
    memcpy((void*)new_frame, (void*)old_frame, 4096);

    pt[pt_index] = new_frame | PAGE_PRESENT | PAGE_WRITE | PAGE_USER;
    __asm__ volatile ("invlpg (%0)" : : "r"(faulting_address) : "memory");

    /* Known limitation, documented rather than engineered around: the
     * old shared frame is never freed here, even once every COW
     * reference to it has resolved into private copies elsewhere - no
     * reference counting exists (this kernel's PMM tracks free/used
     * per frame, nothing more). A bounded leak (exactly one frame per
     * page a fork()'d process ever writes to), not an unbounded one,
     * but a real one. See PROGRESS.md. */
    return true;
}

static void page_fault_handler(registers_t* regs) {
    uint32_t faulting_address = read_cr2();

    /* Intel SDM Vol 3A, 4.7: page-fault error code bit layout. */
    bool present  = regs->err_code & 0x1;
    bool write    = regs->err_code & 0x2;
    bool user     = regs->err_code & 0x4;
    bool reserved = regs->err_code & 0x8;

    if (present && write && try_resolve_cow_fault(faulting_address)) {
        return; /* resolved - the faulting instruction will be
                    re-executed automatically and succeed this time */
    }

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

/* IMPORTANT CAVEAT shared by every function below: they treat a
 * physical frame address returned by pmm_alloc_frame() as a directly
 * usable pointer (and CR3 value) with no translation step. That's
 * only true because NovaOS has no "temporarily map an arbitrary
 * physical page" mechanism yet (no recursive page-directory trick, no
 * kmap-style API), and pmm_alloc_frame()'s bitmap scan happens to hand
 * out low-numbered (and therefore already identity-mapped, since
 * paging_init() mapped 0-64MB) frames first for the small number of
 * allocations this phase makes. It is not a general guarantee - a
 * long-running system that had fragmented or exhausted low memory
 * could get a frame above 64MB here and silently misbehave. Tracked
 * as a real limitation in PROGRESS.md, not a hidden assumption. */

uint32_t paging_kernel_directory_phys(void) {
    return (uint32_t)page_directory;
}

uint32_t paging_create_address_space(void) {
    uint32_t new_pd_phys = pmm_alloc_frame();
    if (new_pd_phys == 0) {
        kernel_panic("paging_create_address_space: out of memory");
    }

    uint32_t* new_pd = (uint32_t*)new_pd_phys;
    /* sizeof(page_directory) is exactly 4096 bytes (1024 * 4-byte
     * entries) - exactly one frame, conveniently. Copying the whole
     * thing (rather than just the "kernel half") also happens to copy
     * every not-yet-used upper entry, which is already all zero/not-
     * present, so this is correct either way. */
    memcpy(new_pd, page_directory, sizeof(page_directory));

    return new_pd_phys;
}

bool paging_map_page(uint32_t* page_directory_ptr, uint32_t virt_addr,
                      uint32_t phys_addr, uint32_t flags) {
    uint32_t pd_index = virt_addr >> 22;
    uint32_t pt_index = (virt_addr >> 12) & 0x3FF;

    uint32_t* table;
    if (!(page_directory_ptr[pd_index] & PAGE_PRESENT)) {
        uint32_t new_pt_phys = pmm_alloc_frame();
        if (new_pt_phys == 0) {
            return false;
        }
        table = (uint32_t*)new_pt_phys;
        memset(table, 0, 4096);
        page_directory_ptr[pd_index] =
            new_pt_phys | PAGE_PRESENT | PAGE_WRITE | PAGE_USER;
    } else {
        table = (uint32_t*)(page_directory_ptr[pd_index] & 0xFFFFF000u);
    }

    table[pt_index] = (phys_addr & 0xFFFFF000u) | (flags & 0xFFFu);
    return true;
}

void paging_switch_address_space(uint32_t page_directory_phys) {
    __asm__ volatile ("mov %0, %%cr3" : : "r"(page_directory_phys)
                       : "memory");
}
