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

/* Phase 83: a page of SHARED MEMORY (kernel/rust/shm.rs): the same
 * physical frame is mapped, writable, into several processes at once.
 * Every piece of code that assumed user pages are private must treat it
 * differently, which is what this bit is for:
 *   - fork() copies the entry verbatim into the child instead of turning
 *     both sides copy-on-write (they must keep seeing each other's writes);
 *   - process teardown must NOT free the frame - it belongs to the shared
 *     object, whose reference counts decide when it goes.
 * Bit 10 is one of the three the CPU ignores and leaves to the OS (see
 * PAGE_COW above). Never set together with PAGE_COW. */
#define PAGE_SHM     0x400

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

/* The size of the kernel's boot-time identity map (virtual == physical
 * for 0 .. this). Exported (not just a private constant in paging.c)
 * because code that allocates a frame with pmm_alloc_frame() and then
 * touches it through its physical address - zeroing a page about to be
 * mapped into user space, say - is only safe if that frame is below this
 * line; the PMM tracks ALL of RAM, so on a machine that has used more
 * than this much, a freshly allocated frame can be one the kernel has
 * no mapping for, and touching it is a page fault the kernel treats as
 * fatal. paging.c asserts this matches IDENTITY_MAP_TABLES. */
#define PAGING_IDENTITY_MAP_BYTES (64u * 1024u * 1024u)

/* Phase 81: is page-directory entry `pde` (the one at `index` of some
 * process's directory) one of the KERNEL'S OWN shared page tables,
 * rather than a page table that process owns?
 *
 * Every process directory starts as a copy of the kernel's, so entries
 * 0-15 (the 64MB identity map) point at page tables that are shared by
 * every process and must never be freed, rewritten or made
 * copy-on-write on a process's behalf. This is the question
 * free_user_address_space(), cow_share_address_space() and
 * paging_user_range_ok() all need answered - and it is answered by
 * comparing the page-table FRAME ADDRESS (bits 31:12) only.
 *
 * It used to be answered by comparing the whole entry for equality
 * (`pd[i] == kernel_pd[i]`), which is wrong in a way that only shows up
 * under memory pressure: the CPU sets the Accessed flag (bit 5) in the
 * PDE it used for a translation, in whichever directory was loaded at
 * the time. A process's copy of an entry therefore gains bit 5 as soon
 * as the kernel touches memory in that 4MB window while running on that
 * process's behalf - while the kernel directory's own copy may never
 * have been touched (nothing at boot reached that window), so it stays
 * clear. The two stop comparing equal, the shared page table looks
 * process-owned, and reaping the process freed every frame it mapped -
 * the kernel's own memory - and then the page table itself. Found by
 * the Phase 81 framebuffer conformance test, whose multi-megabyte
 * surfaces were the first workload to push allocations past 20MB; see
 * PROGRESS.md for the full account. Only the frame address says which
 * table an entry points at, so only the frame address is compared. */
bool paging_pde_is_kernel_shared(uint32_t pde, uint32_t index);

/* Phase 81: removes the mapping for one 4KB page and returns the old
 * page-table entry (so the caller can see the frame, and whether it
 * was a copy-on-write page it must NOT free - see the COW notes in
 * process.c's free_user_address_space()), or 0 if nothing was mapped.
 * Flushes the TLB entry if `page_directory` is the one currently loaded
 * in CR3. Does not free the frame and does not free an emptied page
 * table (those are reclaimed when the address space is torn down). */
uint32_t paging_unmap_page(uint32_t* page_directory, uint32_t virt_addr);

/* Phase 81: may the kernel safely read (or, with for_write, write)
 * `len` bytes at user address `addr` on behalf of the CURRENT process?
 * Walks the loaded page tables and requires every page in the range to
 * be present, user-accessible, and PROCESS-OWNED memory - so unmapped
 * addresses, ranges that wrap past 4GB, and pointers into the kernel's
 * shared identity map (including NULL) are all rejected, instead of
 * becoming a kernel-mode page fault (which this kernel treats as
 * fatal - one bad pointer from ring 3 would otherwise be a way to
 * panic the machine).
 *
 * What this does NOT claim: it does not make kernel memory safe from
 * ring 3. The identity map is deliberately PAGE_USER (documented in
 * paging.c's build_identity_map(), unchanged since Phase 4), so a
 * ring-3 program can still read and write kernel memory directly with
 * ordinary loads and stores; closing that is the per-process-isolation
 * work PROGRESS.md has tracked since Phase 3, far outside this phase.
 * What the check buys is that SYSCALLS never do it on a program's
 * behalf by accident or confusion - a wild pointer becomes -EFAULT
 * instead of a silent kernel write - and that their contract does not
 * silently change when that isolation work lands. A consequence worth
 * stating: code running in ring 3 FROM the kernel image (the in-kernel
 * demo tasks) cannot pass pointers to syscalls that use this check,
 * since everything it owns lives in the shared map. None does.
 *
 * for_write additionally requires each page be writable. A page that
 * is read-only only because it is copy-on-write (shared with a
 * fork()'d relative) is NOT refused: it is resolved right here, giving
 * this process its private copy first. That is required for
 * correctness, not a convenience: this kernel runs with CR0.WP clear,
 * so a ring-0 store to a read-only page does not fault - it silently
 * modifies the frame every sharer sees, which would let a syscall's
 * output scribble on the parent's (or child's) memory.
 *
 * len == 0 is accepted for any address. Intended to be called in
 * syscall context, where CR3 is the calling process's page directory. */
bool paging_user_range_ok(uint32_t addr, uint32_t len, bool for_write);

/* Loads CR3. Called by the scheduler on every context switch. */
void paging_switch_address_space(uint32_t page_directory_phys);

/* The kernel's own page directory's physical address - what every
 * kernel task's process_t.page_directory_phys is set to, since kernel
 * tasks don't get (or need) a private address space. */
uint32_t paging_kernel_directory_phys(void);


/* Phase 86: how many user-accessible pages the address space whose page
 * directory is at physical address `pd_phys` maps. Walks the directory and
 * every present page table; only pages marked PAGE_USER count, which excludes
 * the kernel's own identity mapping that every address space shares. Used by
 * the memory limit (rlimit.c) - a real count of what the process can touch,
 * not a counter that could drift. */
uint32_t paging_count_user_pages(uint32_t pd_phys);

#endif
