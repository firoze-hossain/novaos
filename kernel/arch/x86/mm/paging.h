#ifndef ARCH_X86_MM_PAGING_H
#define ARCH_X86_MM_PAGING_H

#include "../../../include/types.h"

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

#endif
