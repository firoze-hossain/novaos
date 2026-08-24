/*
 * elf.c - minimal ELF32 loader (see elf.h for scope)
 */
#include "elf.h"
#include "../arch/x86/mm/pmm.h"
#include "../arch/x86/mm/paging.h"
#include "../lib/string.h"
#include "../include/kernel.h"

#define EI_NIDENT 16

#define ELFMAG0 0x7F
#define ELFMAG1 'E'
#define ELFMAG2 'L'
#define ELFMAG3 'F'

#define ELFCLASS32 1
#define ELFDATA2LSB 1

#define ET_EXEC 2
#define EM_386  3

#define PT_LOAD 1

#define PF_X 0x1
#define PF_W 0x2
#define PF_R 0x4

typedef struct __attribute__((packed)) {
    uint8_t e_ident[EI_NIDENT];
    uint16_t e_type;
    uint16_t e_machine;
    uint32_t e_version;
    uint32_t e_entry;
    uint32_t e_phoff;
    uint32_t e_shoff;
    uint32_t e_flags;
    uint16_t e_ehsize;
    uint16_t e_phentsize;
    uint16_t e_phnum;
    uint16_t e_shentsize;
    uint16_t e_shnum;
    uint16_t e_shstrndx;
} elf32_header_t;

typedef struct __attribute__((packed)) {
    uint32_t p_type;
    uint32_t p_offset;
    uint32_t p_vaddr;
    uint32_t p_paddr;
    uint32_t p_filesz;
    uint32_t p_memsz;
    uint32_t p_flags;
    uint32_t p_align;
} elf32_phdr_t;

bool elf_validate(const uint8_t* data, uint32_t size) {
    if (size < sizeof(elf32_header_t)) {
        return false;
    }
    const elf32_header_t* hdr = (const elf32_header_t*)data;

    if (hdr->e_ident[0] != ELFMAG0 || hdr->e_ident[1] != ELFMAG1 ||
        hdr->e_ident[2] != ELFMAG2 || hdr->e_ident[3] != ELFMAG3) {
        return false;
    }
    if (hdr->e_ident[4] != ELFCLASS32 || hdr->e_ident[5] != ELFDATA2LSB) {
        return false; /* not 32-bit little-endian - the only kind this
                          kernel's own architecture can run */
    }
    if (hdr->e_type != ET_EXEC) {
        return false; /* statically-linked, non-PIE only - see elf.h */
    }
    if (hdr->e_machine != EM_386) {
        return false;
    }
    if (hdr->e_phoff == 0 || hdr->e_phnum == 0) {
        return false; /* no program headers - nothing to load */
    }
    return true;
}

bool elf_load(const uint8_t* data, uint32_t size,
              uint32_t page_directory_phys, uint32_t* out_entry_point) {
    if (!elf_validate(data, size)) {
        return false;
    }
    const elf32_header_t* hdr = (const elf32_header_t*)data;
    uint32_t* pd = (uint32_t*)page_directory_phys;

    if ((uint64_t)hdr->e_phoff +
            (uint64_t)hdr->e_phnum * sizeof(elf32_phdr_t) >
        size) {
        kernel_log("[FAULT] elf_load: program header table runs past "
                   "end of file\n");
        return false;
    }

    for (uint16_t i = 0; i < hdr->e_phnum; i++) {
        const elf32_phdr_t* ph = (const elf32_phdr_t*)(data + hdr->e_phoff +
                                                         (uint32_t)i *
                                                             sizeof(*ph));
        if (ph->p_type != PT_LOAD) {
            continue; /* PT_DYNAMIC/PT_INTERP/PT_NOTE/etc - not needed
                          for a static, non-PIE executable */
        }
        if (ph->p_filesz > ph->p_memsz) {
            kernel_log("[FAULT] elf_load: segment filesz > memsz\n");
            return false;
        }
        if ((uint64_t)ph->p_offset + (uint64_t)ph->p_filesz > size) {
            kernel_log("[FAULT] elf_load: segment runs past end of "
                       "file\n");
            return false;
        }

        uint32_t seg_start = ph->p_vaddr & 0xFFFFF000u;
        uint32_t seg_end = (ph->p_vaddr + ph->p_memsz + 0xFFFu) & 0xFFFFF000u;
        uint32_t flags = PAGE_PRESENT | PAGE_USER;
        if (ph->p_flags & PF_W) {
            flags |= PAGE_WRITE;
        }

        for (uint32_t page_addr = seg_start; page_addr < seg_end;
             page_addr += 4096) {
            uint32_t frame = pmm_alloc_frame();
            if (frame == 0) {
                kernel_log("[FAULT] elf_load: out of physical memory\n");
                return false;
            }
            paging_map_page(pd, page_addr, frame, flags);

            /* Frames live in identity-mapped low memory (same
             * reasoning as every DMA buffer and user stack elsewhere
             * in this tree - virtual address equals physical address
             * there), so this kernel-side code can zero and copy into
             * them directly through that identity mapping rather than
             * needing to switch address spaces first. */
            memset((void*)frame, 0, 4096);
        }

        /* Copy the file's bytes for this segment (p_filesz of them;
         * anything beyond that up to p_memsz - typically .bss - stays
         * zeroed from the memset above) into the now-mapped pages,
         * one physical frame at a time, since a segment's data may
         * span multiple non-contiguous frames while the source file
         * buffer is one contiguous block. */
        uint32_t bytes_remaining = ph->p_filesz;
        uint32_t file_pos = ph->p_offset;
        uint32_t dest_vaddr = ph->p_vaddr;

        while (bytes_remaining > 0) {
            uint32_t page_base = dest_vaddr & 0xFFFFF000u;
            uint32_t page_offset = dest_vaddr - page_base;
            uint32_t pd_index = page_base >> 22;
            uint32_t pt_index = (page_base >> 12) & 0x3FFu;
            uint32_t* pt = (uint32_t*)(pd[pd_index] & 0xFFFFF000u);
            uint32_t frame = pt[pt_index] & 0xFFFFF000u;

            uint32_t chunk = 4096 - page_offset;
            if (chunk > bytes_remaining) {
                chunk = bytes_remaining;
            }
            memcpy((void*)(frame + page_offset), data + file_pos, chunk);

            bytes_remaining -= chunk;
            file_pos += chunk;
            dest_vaddr += chunk;
        }
    }

    *out_entry_point = hdr->e_entry;
    return true;
}
