#ifndef ARCH_X86_MM_HEAP_H
#define ARCH_X86_MM_HEAP_H

#include "../../../include/types.h"

/* Phase 2 deliberately ships a simple first-fit free-list allocator
 * over a fixed-size static arena rather than a physical-memory-map-
 * aware allocator: NovaOS doesn't parse the Multiboot memory map yet
 * (that, plus paging, is tracked as a Phase 3 follow-up in PROGRESS.md).
 * The interface below is the one later allocators will keep, so callers
 * don't need to change when the implementation does. */
void heap_init(void);

void* kmalloc(size_t size);
void  kfree(void* ptr);

typedef struct heap_stats {
    size_t total_bytes;
    size_t used_bytes;
    size_t free_bytes;
} heap_stats_t;

void heap_get_stats(heap_stats_t* out);

#endif
