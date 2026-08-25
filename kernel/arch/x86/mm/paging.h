#ifndef ARCH_X86_MM_PAGING_H
#define ARCH_X86_MM_PAGING_H

#include "../../../include/types.h"

#define PAGE_PRESENT 0x1
#define PAGE_WRITE   0x2
#define PAGE_USER    0x4
/* Phase 27: bits 9-11 of a page table entry are explicitly ignored by
 * the CPU and reserved for OS use - this marks a page shared between
 * a fork()'d parent and child, read-only in both until whichever one
 * writes to it first triggers a copy (see paging.c's page fault
 * handler). Not a real hardware feature; purely this kernel's own
 * bookkeeping. */
#define PAGE_COW     0x200

/* Identity-maps physical (== virtual) addresses 0-64MB with static,
 * boot-time page tables and enables paging (CR0.PG). Also registers
 * the page-fault handler (vector 14) so a bad access gets a clean,
 * diagnosable panic instead of a triple fault.
 *
 * Why a fixed 64MB rather than "map all detected RAM": the page
 * tables for the initial identity map are static arrays sized at
 * compile time specifically so paging_init() has no dependency on the
 * heap or PMM being ready yet (chicken-and-egg: you need *some*
 * mapped, known-good memory before you can safely allocate more).
 * 64MB is comfortably more than this kernel, its heap arena, and its
 * driver buffers currently need - see PROGRESS.md for the Phase 4+
 * follow-up (per-process address spaces allocated dynamically via the
 * PMM, once that's needed). */
void paging_init(void);

/* --- Phase 5: per-process address spaces --- */

/* Allocates a fresh page directory (via the PMM - it must be a whole,
 * page-aligned physical frame) and copies the kernel's own page
 * directory into it, so every process's address space maps the same
 * kernel code/data/heap range at the same addresses. This is required,
 * not optional: interrupts and syscalls run kernel code using
 * whichever CR3 happens to be loaded at the time (there's no CR3
 * switch on a ring transition, only on a process-to-process context
 * switch) - if the kernel weren't mapped in a process's own
 * directory, the very first interrupt after switching to it would
 * fault on the kernel's own code.
 *
 * Returns the new page directory's physical address (usable directly
 * as a virtual pointer too, and as the value to load into CR3 - see
 * the important caveat in the .c file about why that's true here but
 * won't always be). */
uint32_t paging_create_address_space(void);

/* Maps one 4KB page: virt_addr -> phys_addr, with the given flags
 * (PAGE_PRESENT/PAGE_WRITE/PAGE_USER), into the given page directory -
 * allocating a new page table via the PMM first if that directory
 * doesn't have one for this range yet. `page_directory` must be a
 * directly-dereferenceable pointer (see the .c file's caveat) rather
 * than just any physical address. */
bool paging_map_page(uint32_t* page_directory, uint32_t virt_addr,
                      uint32_t phys_addr, uint32_t flags);

/* Loads CR3. Called by the scheduler on every context switch. */
void paging_switch_address_space(uint32_t page_directory_phys);

/* The kernel's own page directory's physical address - what every
 * kernel task's process_t.page_directory_phys is set to, since kernel
 * tasks don't get (or need) a private address space. */
uint32_t paging_kernel_directory_phys(void);

#endif
