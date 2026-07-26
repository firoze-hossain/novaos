#ifndef ARCH_X86_MM_PMM_H
#define ARCH_X86_MM_PMM_H

#include "../../../include/types.h"
#include "../boot/multiboot.h"

#define PMM_FRAME_SIZE 4096

/* Parses the Multiboot memory map to find which physical frames are
 * actually installed RAM, then reserves frame 0 (real-mode IVT/BIOS
 * data) and the frames the kernel image itself occupies
 * ([kernel_start, kernel_end), from the linker script) so nothing else
 * can be handed out on top of running kernel code or data.
 *
 * `magic_valid` should be true only if kernel_main() was actually
 * entered with EAX == MULTIBOOT_BOOTLOADER_MAGIC; with a non-Multiboot
 * (or corrupt) boot, PMM falls back to treating all tracked frames as
 * used, which is conservative but safe - allocation just reports "out
 * of memory" rather than handing out a frame it can't prove is real RAM. */
void pmm_init(const multiboot_info_t* mbi, bool magic_valid);

/* Returns the physical address of a free 4KB frame (and marks it
 * used), or 0 if none are available. 0 is never itself a valid
 * allocatable frame (it's permanently reserved), so 0 unambiguously
 * means failure. */
uint32_t pmm_alloc_frame(void);

void pmm_free_frame(uint32_t phys_addr);

typedef struct pmm_stats {
    uint32_t total_frames;   /* frames PMM is tracking at all */
    uint32_t used_frames;
    uint32_t free_frames;
} pmm_stats_t;

void pmm_get_stats(pmm_stats_t* out);

#endif
