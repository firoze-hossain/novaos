/*
 * pmm.c - Physical Memory Manager
 *
 * A classic bitmap allocator: one bit per 4KB physical frame, 1 = used,
 * 0 = free. Tracks at most the first 1GB of physical RAM (262144
 * frames, a 32KB bitmap) - generous for a hobby OS's current needs
 * (QEMU's default 512MB test config uses under half of that) and a
 * bounded, static allocation instead of something that has to be sized
 * dynamically before a heap exists to size it with.
 *
 * This only decides *which frames are free*; it doesn't map them
 * anywhere. paging.c currently identity-maps a fixed low range with
 * its own static page tables independently of this allocator (see the
 * comment in paging.c for why) - pmm_alloc_frame() exists for future
 * callers that need an arbitrary physical page (additional page
 * tables, DMA buffers, a future per-process address space in Phase 4).
 */
#include "pmm.h"
#include "../../../include/kernel.h"

#define MAX_TRACKED_FRAMES (256u * 1024u) /* 1GB / 4KB */
#define BITMAP_WORDS (MAX_TRACKED_FRAMES / 32)

extern uint32_t kernel_start;
extern uint32_t kernel_end;

static uint32_t bitmap[BITMAP_WORDS];
static uint32_t total_frames = 0;

static inline void bitmap_set(uint32_t frame) {
    bitmap[frame / 32] |= (1u << (frame % 32));
}

static inline void bitmap_clear(uint32_t frame) {
    bitmap[frame / 32] &= ~(1u << (frame % 32));
}

static inline bool bitmap_test(uint32_t frame) {
    return (bitmap[frame / 32] & (1u << (frame % 32))) != 0;
}

static void reserve_range(uint32_t phys_start, uint32_t phys_end) {
    uint32_t first = phys_start / PMM_FRAME_SIZE;
    uint32_t last = (phys_end + PMM_FRAME_SIZE - 1) / PMM_FRAME_SIZE;
    for (uint32_t f = first; f < last && f < MAX_TRACKED_FRAMES; f++) {
        bitmap_set(f);
    }
}

static void free_range(uint32_t phys_start, uint32_t phys_end) {
    uint32_t first = phys_start / PMM_FRAME_SIZE;
    uint32_t last = phys_end / PMM_FRAME_SIZE;
    for (uint32_t f = first; f < last && f < MAX_TRACKED_FRAMES; f++) {
        bitmap_clear(f);
        if (f + 1 > total_frames) {
            total_frames = f + 1;
        }
    }
}

void pmm_init(const multiboot_info_t* mbi, bool magic_valid) {
    /* Start pessimistic: everything is "used" until proven to be
     * available RAM. This means a bad/missing memory map fails safe
     * (pmm_alloc_frame() just always returns 0) instead of handing out
     * a frame that might not be backed by real memory at all. */
    for (uint32_t i = 0; i < BITMAP_WORDS; i++) {
        bitmap[i] = 0xFFFFFFFFu;
    }
    total_frames = 0;

    if (magic_valid && mbi != NULL && (mbi->flags & MULTIBOOT_INFO_MEM_MAP)) {
        uint32_t offset = 0;
        while (offset < mbi->mmap_length) {
            const multiboot_mmap_entry_t* entry =
                (const multiboot_mmap_entry_t*)(mbi->mmap_addr + offset);

            if (entry->type == MULTIBOOT_MEMORY_AVAILABLE) {
                /* Physical addresses/lengths are 64-bit in the spec to
                 * support PAE/huge memory maps; NovaOS is 32-bit
                 * non-PAE, so clamp to 32 bits (see PROGRESS.md - this
                 * is the same limitation that keeps NX off the table). */
                uint32_t start = (uint32_t)(entry->addr & 0xFFFFFFFFu);
                uint64_t end64 = entry->addr + entry->len;
                uint32_t end = (end64 > 0xFFFFFFFFull)
                                   ? 0xFFFFFFFFu
                                   : (uint32_t)end64;
                free_range(start, end);
            }

            offset += entry->size + sizeof(entry->size);
        }
    } else if (magic_valid && mbi != NULL &&
               (mbi->flags & MULTIBOOT_INFO_MEMORY)) {
        /* No memory map, but GRUB at least gave us mem_lower/mem_upper.
         * Treat everything from 1MB to 1MB+mem_upper as available -
         * cruder than the real map, but still correct for QEMU/most
         * real BIOSes since that range is contiguous in practice. */
        free_range(0x100000, 0x100000 + mbi->mem_upper * 1024u);
    }
    /* else: no usable Multiboot info at all - stay fully reserved. */

    /* Now re-reserve what must never be handed out, regardless of what
     * the memory map said was "available": the first 1MB (real-mode
     * IVT, BIOS data area, video memory, and where our own boot code
     * briefly ran) and the kernel image itself. */
    reserve_range(0, 0x100000);
    reserve_range((uint32_t)&kernel_start, (uint32_t)&kernel_end);

    kernel_log("[ OK ] PMM initialized (%d frames tracked, %dMB)\n",
               (int)total_frames, (int)(total_frames * PMM_FRAME_SIZE / (1024 * 1024)));
}

uint32_t pmm_alloc_frame(void) {
    for (uint32_t f = 0; f < total_frames; f++) {
        if (!bitmap_test(f)) {
            bitmap_set(f);
            return f * PMM_FRAME_SIZE;
        }
    }
    return 0; /* out of memory */
}

void pmm_free_frame(uint32_t phys_addr) {
    uint32_t frame = phys_addr / PMM_FRAME_SIZE;
    if (frame < MAX_TRACKED_FRAMES) {
        bitmap_clear(frame);
    }
}

void pmm_get_stats(pmm_stats_t* out) {
    if (out == NULL) {
        return;
    }
    out->total_frames = total_frames;
    out->used_frames = 0;
    for (uint32_t f = 0; f < total_frames; f++) {
        if (bitmap_test(f)) {
            out->used_frames++;
        }
    }
    out->free_frames = total_frames - out->used_frames;
}
