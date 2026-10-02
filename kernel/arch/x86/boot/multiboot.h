#ifndef ARCH_X86_BOOT_MULTIBOOT_H
#define ARCH_X86_BOOT_MULTIBOOT_H

#include "../../../include/types.h"

#define MULTIBOOT_BOOTLOADER_MAGIC 0x2BADB002

/* Bit positions in multiboot_info_t.flags that tell us which fields
 * GRUB actually filled in. */
#define MULTIBOOT_INFO_MEMORY   0x00000001
#define MULTIBOOT_INFO_MEM_MAP  0x00000040
/* Phase 79: set only if GRUB actually honored this kernel's own
 * video-mode request (multiboot.asm's header) - the framebuffer_*
 * fields below are meaningless, not merely zero, when this bit is
 * clear. See kernel/drivers/video/vbe.c's own vbe_init() for the one
 * place this is actually checked, and its own fallback when it
 * isn't. */
#define MULTIBOOT_INFO_FRAMEBUFFER_INFO 0x00001000

#define MULTIBOOT_FRAMEBUFFER_TYPE_INDEXED 0
#define MULTIBOOT_FRAMEBUFFER_TYPE_RGB     1
#define MULTIBOOT_FRAMEBUFFER_TYPE_EGA_TEXT 2

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

    /* Phase 79: the remaining fields this project's own struct used
     * to stop short of - drives, config table, boot loader name, APM
     * table, VBE/framebuffer info - added now that kernel/drivers/
     * video/vbe.c actually needs the framebuffer_* ones. Every field
     * and its exact byte order below is verified against GRUB's own
     * canonical multiboot.h (not reconstructed from a secondary
     * description) - a wrong offset here wouldn't just misread one
     * value, it would silently misalign every field after it, up to
     * and including this kernel treating a garbage value as a real
     * physical framebuffer address and writing to it. drives_length/
     * drives_addr/config_table/boot_loader_name/apm_table/vbe_* are
     * still unread by anything in this kernel - kept only because
     * skipping them would misalign the framebuffer_* fields that
     * follow, which the real struct places after them, not instead of
     * them. */
    uint32_t drives_length;
    uint32_t drives_addr;
    uint32_t config_table;
    uint32_t boot_loader_name;
    uint32_t apm_table;
    uint32_t vbe_control_info;
    uint32_t vbe_mode_info;
    uint16_t vbe_mode;
    uint16_t vbe_interface_seg;
    uint16_t vbe_interface_off;
    uint16_t vbe_interface_len;

    uint64_t framebuffer_addr;   /* physical address, real struct is
                                     64-bit even though this kernel's
                                     own 32-bit addressing only ever
                                     uses the low 32 bits of it */
    uint32_t framebuffer_pitch;  /* bytes per scanline - NOT always
                                     width*bpp/8; real hardware may pad
                                     each row, so every pixel write
                                     must go through this, never a
                                     computed stride */
    uint32_t framebuffer_width;
    uint32_t framebuffer_height;
    uint8_t  framebuffer_bpp;
    uint8_t  framebuffer_type;   /* MULTIBOOT_FRAMEBUFFER_TYPE_* above -
                                     only RGB (1) is one this kernel
                                     reads meaning from; see vbe_init()
                                     for why INDEXED/EGA_TEXT fall back
                                     the same as no framebuffer at all */
    /* Real struct: a union here (palette_addr+num_colors for indexed
     * color, OR these six fields for direct RGB color) - only the RGB
     * variant is modeled, matching this project's own established
     * "model only what we actually read" convention (e.g. `syms[4]`
     * above, kept unparsed on purpose) - reading these six bytes when
     * framebuffer_type is actually INDEXED (0) would just read the
     * other variant's own bytes reinterpreted, which is exactly why
     * vbe_init() never does that: it checks framebuffer_type == RGB
     * first and ignores these fields entirely otherwise. */
    uint8_t  framebuffer_red_field_position;
    uint8_t  framebuffer_red_mask_size;
    uint8_t  framebuffer_green_field_position;
    uint8_t  framebuffer_green_mask_size;
    uint8_t  framebuffer_blue_field_position;
    uint8_t  framebuffer_blue_mask_size;
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
