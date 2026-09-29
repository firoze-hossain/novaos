#ifndef TASK_ELF_H
#define TASK_ELF_H

#include "../include/types.h"

/* Minimal ELF32 loader: statically-linked, non-PIE, ET_EXEC binaries
 * only. Enough to load and run a real, independently-compiled
 * executable - the load-bearing feature everything else in "run real
 * software on NovaOS" depends on - without the added scope of dynamic
 * linking (no PT_INTERP/PT_DYNAMIC handling) or position-independent
 * executables (no relocation processing). See PROGRESS.md for the
 * full scope note. */

/* Validates the ELF header: magic, 32-bit class, little-endian data,
 * EM_386 machine, ET_EXEC type. Does not load anything - just answers
 * "is this a file elf_load() should even attempt." */
bool elf_validate(const uint8_t* data, uint32_t size);

/* Loads every PT_LOAD segment from `data` into the address space
 * given by `page_directory_phys` (a page directory this function maps
 * fresh physical frames into via paging_map_page() - the same
 * primitive process.c already uses for user stacks). Segment
 * permissions (PF_W) are respected for the PAGE_WRITE bit; PF_X
 * (execute) has no CPU-level effect since this kernel's 32-bit
 * non-PAE paging has no NX bit, a limitation documented since Phase 3
 * - not a new gap introduced here. Returns true and fills
 * *out_entry_point with e_entry on success.
 *
 * UNCHANGED by Phase 74's dynamic-linking work below, on purpose: this
 * is still the entire load path for every existing static binary (an
 * ET_EXEC with no PT_DYNAMIC), and stays exactly as proven correct
 * before - zero shared code, zero shared risk with the new path. */
bool elf_load(const uint8_t* data, uint32_t size,
              uint32_t page_directory_phys, uint32_t* out_entry_point);

/* ------------------------------------------------------------------
 * Phase 74: dynamic linking. See kernel/rust/dynlink.rs's own module
 * comment for the full design (eager/BIND_NOW binding, which
 * relocation types are supported and why, and what is deliberately
 * out of scope). This header only adds what elf.c needs to hand a
 * loaded image's shape to that Rust engine and to load a SECOND kind
 * of image - a shared library (ET_DYN), mapped at a runtime-chosen
 * bias rather than its own fixed link address - alongside the
 * existing ET_EXEC-only elf_load() above.
 * ------------------------------------------------------------------ */

/* One PT_LOAD segment's (link-time vaddr, file offset, file size) -
 * field-for-field, in the same order, as kernel/rust/dynlink.rs's own
 * `#[repr(C)] struct LoadSeg`. Kept as its own named type rather than
 * three loose uint32_t's so the two sides of that FFI boundary can
 * only go out of sync by an actual struct-shape mismatch, which the
 * compiler catches, not by an argument-order slip, which it can't. */
typedef struct {
    uint32_t vaddr;
    uint32_t offset;
    uint32_t filesz;
} elf_load_seg_t;

/* PT_LOAD segments a single ELF image is expected to have. This
 * project's own toolchain output tops out at 4 (text, rodata, data+
 * bss, the tiny PT_DYNAMIC-covering RW segment - see this file's own
 * investigation notes in PROGRESS.md); 8 is real headroom, not a
 * tight fit, and elf_inspect() below fails closed (returns false)
 * rather than silently dropping segments if a file ever has more. */
#define MAX_LOAD_SEGS 8

/* Everything elf_inspect() learns about one ELF image in a single
 * pass over its program headers: where its PT_LOAD segments are (for
 * both mapping them and, later, for kernel/rust/dynlink.rs's own
 * vaddr->file-offset translation), and where (if anywhere) its
 * PT_DYNAMIC segment is. */
typedef struct {
    elf_load_seg_t segs[MAX_LOAD_SEGS];
    uint32_t seg_count;
    bool has_dynamic;
    uint32_t dyn_offset; /* file offset of the .dynamic array */
    uint32_t dyn_filesz;
    uint32_t entry_point;
    bool is_dyn; /* ET_DYN (a shared library) vs ET_EXEC */
} elf_image_t;

/* Validates the ELF header (32-bit, little-endian, EM_386, and either
 * ET_EXEC or ET_DYN - unlike elf_validate() above, which only accepts
 * ET_EXEC) and walks its program headers exactly once, filling
 * *out_image. Does not map or copy anything - this is the shared first
 * step for both the main executable (called in addition to elf_load(),
 * purely to learn whether/where it has a PT_DYNAMIC segment) and every
 * shared library (called before elf_load_segments_biased() below ever
 * touches memory). Returns false on any malformed input, including
 * more than MAX_LOAD_SEGS PT_LOAD segments. */
bool elf_inspect(const uint8_t* data, uint32_t size, elf_image_t* out_image);

/* Maps and copies every PT_LOAD segment from `data` into
 * `page_directory_phys`, exactly as elf_load() does, EXCEPT every
 * segment's virtual address is `bias + p_vaddr` rather than `p_vaddr`
 * alone - what a shared library (built as ET_DYN, so its own p_vaddr
 * values start from a link-time base of 0) needs to actually run at
 * whatever real address the kernel chose to load it at. Pass bias = 0
 * to reproduce elf_load()'s own placement exactly (used nowhere today
 * - elf_load() itself remains the path for the common, non-dynamic
 * case - but correct and available). `image` must already be filled
 * in by a prior elf_inspect() call on the same `data`. */
bool elf_load_segments_biased(const uint8_t* data, uint32_t size,
                               uint32_t page_directory_phys, uint32_t bias,
                               const elf_image_t* image);

#endif
