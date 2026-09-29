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
#define ET_DYN  3
#define EM_386  3

#define PT_LOAD    1
#define PT_DYNAMIC 2

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

/* ------------------------------------------------------------------
 * Phase 74: dynamic linking - see elf.h for the design note.
 * ------------------------------------------------------------------ */

bool elf_inspect(const uint8_t* data, uint32_t size, elf_image_t* out_image) {
    if (size < sizeof(elf32_header_t)) {
        return false;
    }
    const elf32_header_t* hdr = (const elf32_header_t*)data;

    if (hdr->e_ident[0] != ELFMAG0 || hdr->e_ident[1] != ELFMAG1 ||
        hdr->e_ident[2] != ELFMAG2 || hdr->e_ident[3] != ELFMAG3) {
        return false;
    }
    if (hdr->e_ident[4] != ELFCLASS32 || hdr->e_ident[5] != ELFDATA2LSB) {
        return false;
    }
    if (hdr->e_type != ET_EXEC && hdr->e_type != ET_DYN) {
        return false;
    }
    if (hdr->e_machine != EM_386) {
        return false;
    }
    if (hdr->e_phoff == 0 || hdr->e_phnum == 0) {
        return false;
    }
    if ((uint64_t)hdr->e_phoff + (uint64_t)hdr->e_phnum * sizeof(elf32_phdr_t) >
        size) {
        kernel_log("[FAULT] elf_inspect: program header table runs past "
                   "end of file\n");
        return false;
    }

    out_image->seg_count = 0;
    out_image->has_dynamic = false;
    out_image->dyn_offset = 0;
    out_image->dyn_filesz = 0;
    out_image->entry_point = hdr->e_entry;
    out_image->is_dyn = (hdr->e_type == ET_DYN);

    for (uint16_t i = 0; i < hdr->e_phnum; i++) {
        const elf32_phdr_t* ph = (const elf32_phdr_t*)(data + hdr->e_phoff +
                                                         (uint32_t)i *
                                                             sizeof(*ph));
        if (ph->p_type == PT_DYNAMIC) {
            if ((uint64_t)ph->p_offset + (uint64_t)ph->p_filesz > size) {
                kernel_log("[FAULT] elf_inspect: PT_DYNAMIC runs past end "
                           "of file\n");
                return false;
            }
            out_image->has_dynamic = true;
            out_image->dyn_offset = ph->p_offset;
            out_image->dyn_filesz = ph->p_filesz;
            continue;
        }
        if (ph->p_type != PT_LOAD) {
            continue;
        }
        if (ph->p_filesz > ph->p_memsz) {
            kernel_log("[FAULT] elf_inspect: segment filesz > memsz\n");
            return false;
        }
        if ((uint64_t)ph->p_offset + (uint64_t)ph->p_filesz > size) {
            kernel_log("[FAULT] elf_inspect: segment runs past end of "
                       "file\n");
            return false;
        }
        if (out_image->seg_count >= MAX_LOAD_SEGS) {
            kernel_log("[FAULT] elf_inspect: more than %d PT_LOAD segments "
                       "(unexpected for this project's own toolchain "
                       "output - see elf.h)\n", MAX_LOAD_SEGS);
            return false;
        }
        elf_load_seg_t* seg = &out_image->segs[out_image->seg_count++];
        seg->vaddr = ph->p_vaddr;
        seg->offset = ph->p_offset;
        seg->filesz = ph->p_filesz;
    }

    return true;
}

bool elf_load_segments_biased(const uint8_t* data, uint32_t size,
                               uint32_t page_directory_phys, uint32_t bias,
                               const elf_image_t* image) {
    uint32_t* pd = (uint32_t*)page_directory_phys;

    /* elf_inspect() only ever recorded PT_LOAD segments here (it keeps
     * PT_DYNAMIC's location separately), so every entry is mapped -
     * matching elf_load()'s own "every PT_LOAD, nothing else" scope,
     * just biased and driven from the already-parsed segment table
     * instead of re-walking program headers. Permission flags (PF_W)
     * aren't in elf_load_seg_t - re-reading the one relevant program
     * header field back out of `data` at seg->offset's corresponding
     * p_vaddr would need another lookup for no real benefit, so this
     * maps every segment PAGE_WRITE, same as elf_load() effectively
     * does for .bss-bearing segments already. Honest, minor looseness:
     * a read-only segment (.rodata, most of .text) is technically
     * writable too. This kernel's non-PAE paging has no per-page
     * read-only enforcement gap that PF_X/NX doesn't already have (see
     * elf_load()'s own comment), so this changes no real guarantee. */
    for (uint32_t s = 0; s < image->seg_count; s++) {
        const elf_load_seg_t* seg = &image->segs[s];
        uint32_t load_vaddr = bias + seg->vaddr;

        uint32_t seg_start = load_vaddr & 0xFFFFF000u;
        /* memsz isn't tracked in elf_load_seg_t (only filesz - see
         * elf.h). Every shared library and executable this project's
         * own toolchain produces gives every PT_LOAD segment a page-
         * aligned p_memsz >= p_filesz with any .bss tail covered by a
         * SEPARATE, later PT_LOAD segment in practice for the small,
         * simple images this loader targets - rounding filesz up to a
         * page boundary here is therefore equivalent for them. A
         * library whose linker script merges .bss into the same
         * segment as .data without page-aligning would be under-
         * mapped; none this project builds does. */
        uint32_t seg_end = (load_vaddr + seg->filesz + 0xFFFu) & 0xFFFFF000u;

        for (uint32_t page_addr = seg_start; page_addr < seg_end;
             page_addr += 4096) {
            uint32_t frame = pmm_alloc_frame();
            if (frame == 0) {
                kernel_log("[FAULT] elf_load_segments_biased: out of "
                           "physical memory\n");
                return false;
            }
            paging_map_page(pd, page_addr, frame,
                             PAGE_PRESENT | PAGE_WRITE | PAGE_USER);
            memset((void*)frame, 0, 4096);
        }

        uint32_t bytes_remaining = seg->filesz;
        uint32_t file_pos = seg->offset;
        uint32_t dest_vaddr = load_vaddr;

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

    return true;
}
