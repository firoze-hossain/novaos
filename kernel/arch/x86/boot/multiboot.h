#ifndef ARCH_X86_BOOT_MULTIBOOT_H
#define ARCH_X86_BOOT_MULTIBOOT_H

#include "../../../include/types.h"

#define MULTIBOOT_BOOTLOADER_MAGIC 0x2BADB002

/* Bit positions in multiboot_info_t.flags that tell us which fields
 * GRUB actually filled in. We only ever check MEMORY and MEM_MAP. */
#define MULTIBOOT_INFO_MEMORY   0x00000001
#define MULTIBOOT_INFO_MEM_MAP  0x00000040

typedef struct multiboot_info {
    uint32_t flags;

    uint32_t mem_lower; /* KB of usable low memory (below 1MB)  */
    uint32_t mem_upper; /* KB of usable memory starting at 1MB  */

    uint32_t boot_device;
    uint32_t cmdline;
    uint32_t mods_count;
    uint32_t mods_addr;

    uint32_t syms[4]; /* a.out or ELF symbol table info - unused */

    uint32_t mmap_length;
    uint32_t mmap_addr;

    /* Remaining fields (drives, config table, boot loader name,
     * APM table, VBE info) exist in the real structure but NovaOS
     * doesn't read them yet. */
} __attribute__((packed)) multiboot_info_t;

/* One entry in the BIOS-provided memory map that mmap_addr points to.
 * Entries are variable-length in the spec (size + 4 bytes each,
 * `size` doesn't include itself) - always step by `size + 4` bytes,
 * never by sizeof(multiboot_mmap_entry_t). */
typedef struct multiboot_mmap_entry {
    uint32_t size;
    uint64_t addr;
    uint64_t len;
    uint32_t type; /* 1 = available RAM, everything else = reserved/unusable */
} __attribute__((packed)) multiboot_mmap_entry_t;

#define MULTIBOOT_MEMORY_AVAILABLE 1

#endif
