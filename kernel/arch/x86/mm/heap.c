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

void heap_init(void) {
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
        heap_init();
    }
    if (size == 0) {
        return NULL;
    }

    /* 8-byte alignment keeps double/uint64_t fields inside allocations
     * naturally aligned. */
    size = (size + 7) & ~((size_t)7);

    for (block_header_t* b = heap_start; b != NULL; b = b->next) {
        if (b->free && b->size >= size) {
            split_block(b, size);
            b->free = false;
            return (void*)(b + 1);
        }
    }

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
    if (block->magic != HEAP_MAGIC) {
        kernel_panic("Heap corruption detected (bad free)");
        return;
    }
    if (block->free) {
        kernel_panic("Double free detected");
        return;
    }

    block->free = true;
    coalesce();
}

void heap_get_stats(heap_stats_t* out) {
    if (out == NULL) {
        return;
    }
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
}
