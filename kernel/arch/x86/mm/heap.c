/*
 * heap.c - kmalloc()/kfree() over a static arena
 *
 * Classic first-fit free-list allocator. Each block has a small header
 * (size, free flag, and a magic number). The magic number isn't load-
 * bearing for correctness, but it turns a corrupted-heap bug (e.g. a
 * driver writing past the end of an allocation) into an immediate,
 * loud kernel_panic() at the next kfree() instead of a silent,
 * hard-to-diagnose crash somewhere unrelated later on - a small but
 * genuine security/reliability win for very little code.
 */
#include "heap.h"
#include "../../../include/kernel.h"
#include "../../../lib/spinlock.h"

#define HEAP_MAGIC   0x4E4F5641u /* "NOVA" */
#define HEAP_SIZE    (2 * 1024 * 1024)

typedef struct block_header {
    uint32_t magic;
    size_t size;             /* usable size, excluding this header */
    bool free;
    struct block_header* next;
} block_header_t;

static uint8_t heap_arena[HEAP_SIZE] __attribute__((aligned(16)));
static block_header_t* heap_start = NULL;

/* Phase 57: named directly in this project's own release-readiness
 * roadmap ("the heap allocator") - kmalloc()/kfree() walk and mutate
 * this same free list (block sizes, `free` flags, `next` pointers)
 * regardless of which CPU calls them; two CPUs both splitting or
 * coalescing blocks at the same physical instant, unlocked, could
 * easily corrupt the list itself (a lost update to a `next` pointer,
 * not just a double-allocation) - worse than pmm.c's bitmap race,
 * since a corrupted free list can silently hand out overlapping
 * allocations to two unrelated callers. heap_init()'s own lazy-init
 * path inside kmalloc() below is not itself racy in practice: every
 * CPU capable of calling kmalloc() at all only exists after Phase 56's
 * own SMP bring-up, which runs from kernel_late_init() - well after
 * kernel_early_init()'s own explicit heap_init() call
 * (kernel/init/main.c) has already run to completion on the BSP alone. */
static spinlock_t heap_lock;

void heap_init(void) {
    spinlock_init(&heap_lock);
    heap_start = (block_header_t*)heap_arena;
    heap_start->magic = HEAP_MAGIC;
    heap_start->size  = HEAP_SIZE - sizeof(block_header_t);
    heap_start->free  = true;
    heap_start->next  = NULL;
}

static void split_block(block_header_t* block, size_t size) {
    size_t remaining = block->size - size;
    if (remaining <= sizeof(block_header_t) + 16) {
        return; /* not worth splitting */
    }

    block_header_t* new_block =
        (block_header_t*)((uint8_t*)(block + 1) + size);
    new_block->magic = HEAP_MAGIC;
    new_block->size  = remaining - sizeof(block_header_t);
    new_block->free  = true;
    new_block->next  = block->next;

    block->size = size;
    block->next = new_block;
}

void* kmalloc(size_t size) {
    if (heap_start == NULL) {
        heap_init(); /* see heap_lock's own comment above - safe,
                        unlocked, only ever true this early in boot */
    }
    if (size == 0) {
        return NULL;
    }

    /* 8-byte alignment keeps double/uint64_t fields inside allocations
     * naturally aligned. */
    size = (size + 7) & ~((size_t)7);

    uint32_t flags = spinlock_acquire(&heap_lock);
    for (block_header_t* b = heap_start; b != NULL; b = b->next) {
        if (b->free && b->size >= size) {
            split_block(b, size);
            b->free = false;
            spinlock_release(&heap_lock, flags);
            return (void*)(b + 1);
        }
    }
    spinlock_release(&heap_lock, flags);

    return NULL; /* out of memory */
}

static void coalesce(void) {
    for (block_header_t* b = heap_start; b != NULL && b->next != NULL; ) {
        if (b->free && b->next->free) {
            b->size += sizeof(block_header_t) + b->next->size;
            b->next = b->next->next;
            /* re-check the same block in case another free neighbor follows */
        } else {
            b = b->next;
        }
    }
}

void kfree(void* ptr) {
    if (ptr == NULL) {
        return;
    }

    block_header_t* block = ((block_header_t*)ptr) - 1;
    /* The magic/free checks below run *before* acquiring heap_lock -
     * deliberately: both are read-only checks of memory this specific
     * caller already privately owns (its own allocation's header),
     * not the shared free-list structure itself, so they can't race
     * another CPU's concurrent kmalloc()/kfree() on a *different*
     * block. Only the actual mutation (marking free + coalescing,
     * which does walk/rewrite the shared list) needs the lock. */
    if (block->magic != HEAP_MAGIC) {
        kernel_panic("Heap corruption detected (bad free)");
        return;
    }
    if (block->free) {
        kernel_panic("Double free detected");
        return;
    }

    uint32_t flags = spinlock_acquire(&heap_lock);
    block->free = true;
    coalesce();
    spinlock_release(&heap_lock, flags);
}

void heap_get_stats(heap_stats_t* out) {
    if (out == NULL) {
        return;
    }
    uint32_t flags = spinlock_acquire(&heap_lock);
    out->total_bytes = HEAP_SIZE;
    out->used_bytes = 0;
    out->free_bytes = 0;

    for (block_header_t* b = heap_start; b != NULL; b = b->next) {
        if (b->free) {
            out->free_bytes += b->size;
        } else {
            out->used_bytes += b->size;
        }
    }
    spinlock_release(&heap_lock, flags);
}
