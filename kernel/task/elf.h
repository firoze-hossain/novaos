#ifndef TASK_ELF_H
#define TASK_ELF_H

#include "../include/types.h"

/* Minimal ELF32 loader: statically-linked, non-PIE, ET_EXEC binaries
 * only. Enough to load and run a real, independently-compiled
 * executable - the load-bearing feature everything else in "run real
 * software on NovaOS" depends on - without the added scope of dynamic
 * linking (no PT_INTERP/PT_DYNAMIC handling) or position-independent
 * executables (no relocation processing). See PROGRESS.md for the
 * full scope note. */

/* Validates the ELF header: magic, 32-bit class, little-endian data,
 * EM_386 machine, ET_EXEC type. Does not load anything - just answers
 * "is this a file elf_load() should even attempt." */
bool elf_validate(const uint8_t* data, uint32_t size);

/* Loads every PT_LOAD segment from `data` into the address space
 * given by `page_directory_phys` (a page directory this function maps
 * fresh physical frames into via paging_map_page() - the same
 * primitive process.c already uses for user stacks). Segment
 * permissions (PF_W) are respected for the PAGE_WRITE bit; PF_X
 * (execute) has no CPU-level effect since this kernel's 32-bit
 * non-PAE paging has no NX bit, a limitation documented since Phase 3
 * - not a new gap introduced here. Returns true and fills
 * *out_entry_point with e_entry on success. */
bool elf_load(const uint8_t* data, uint32_t size,
              uint32_t page_directory_phys, uint32_t* out_entry_point);

#endif
