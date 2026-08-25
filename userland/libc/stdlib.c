/*
 * stdlib.c - malloc/free (a small first-fit allocator over SYS_SBRK),
 * atoi, and exit()
 */
#include "stdlib.h"
#include "novasys.h"

/* First-fit allocator, the same overall design as the kernel's own
 * heap.c, written fresh here since userland code has no way to call
 * kernel code directly - only through syscalls. Each allocation has
 * an 8-byte header living just before the pointer malloc() returns.
 *
 * Known limitations, honest rather than hidden: free() never merges
 * adjacent free blocks back together (no coalescing), so a
 * alloc/free/alloc-different-size pattern can fragment the heap over
 * a long-running program's lifetime; and the heap only ever grows
 * (via SYS_SBRK) - freed memory is reused within this allocator's own
 * free list but never actually returned to the kernel. Both are
 * standard simplifications for a first malloc implementation, not
 * oversights. */
typedef struct block_header {
    unsigned int size;
    int free;
    struct block_header* next;
} block_header_t;

static block_header_t* heap_head = NULL;

static unsigned int align_up(unsigned int value, unsigned int align) {
    return (value + align - 1) & ~(align - 1);
}

void* malloc(size_t size) {
    unsigned int aligned_size = align_up((unsigned int)size, 8);

    block_header_t* prev = NULL;
    for (block_header_t* cur = heap_head; cur != NULL; cur = cur->next) {
        if (cur->free && cur->size >= aligned_size) {
            cur->free = 0;
            return (void*)(cur + 1);
        }
        prev = cur;
    }

    unsigned int total = (unsigned int)sizeof(block_header_t) + aligned_size;
    void* mem = sys_sbrk((int)total);
    if (mem == (void*)-1) {
        return NULL; /* out of memory */
    }

    block_header_t* new_block = (block_header_t*)mem;
    new_block->size = aligned_size;
    new_block->free = 0;
    new_block->next = NULL;

    if (prev != NULL) {
        prev->next = new_block;
    } else {
        heap_head = new_block;
    }

    return (void*)(new_block + 1);
}

void free(void* ptr) {
    if (ptr == NULL) {
        return;
    }
    block_header_t* block = ((block_header_t*)ptr) - 1;
    block->free = 1;
}

int atoi(const char* s) {
    int result = 0;
    int sign = 1;

    while (*s == ' ') {
        s++;
    }
    if (*s == '-') {
        sign = -1;
        s++;
    } else if (*s == '+') {
        s++;
    }
    while (*s >= '0' && *s <= '9') {
        result = result * 10 + (*s - '0');
        s++;
    }
    return result * sign;
}

void exit(int code) {
    sys_exit(code); /* never returns */
    __builtin_unreachable();
}
