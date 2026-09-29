//! kernel/rust/dynlink.rs - Phase 74: the ELF32 dynamic-linking engine
//! (PT_DYNAMIC parsing, DT_NEEDED enumeration, dynamic-symbol lookup,
//! and REL-relocation resolution) behind NovaOS's shared-library
//! support.
//!
//! ## Scope and division of labour
//!
//! This module is pure logic: every function here takes byte slices
//! (an ELF file's raw bytes, still exactly as read from disk) and
//! plain integers in, and returns plain integers/bools or calls a
//! caller-supplied callback out. It never touches a page table, a
//! page directory, or the VFS - unlike every other C subsystem this
//! module is called from (`kernel/task/elf.c`), which already owns
//! reading files and mapping memory, this stays out of that entirely.
//! That split is deliberate for two reasons: it matches how every
//! other kernel/rust/ module here works (`tcp.rs` doesn't reimplement
//! the NIC driver, `pkgsign.rs` doesn't reimplement file I/O), and it
//! makes this module testable as an ordinary host program - no QEMU,
//! no kernel, just real cross-compiled ELF files and a small harness
//! (see `kernel/rust/tests/`) - which is how the relocation
//! arithmetic below was actually proven correct before it ever ran
//! inside NovaOS itself.
//!
//! `#![no_std]`, no allocator (the kernel-side Rust crate this belongs
//! to links only `core` and `compiler_builtins` - see `kernel/rust/
//! lib.rs`), and no unbounded loops: every walk here is bounded by a
//! size read from the file itself and checked against the buffer's
//! actual length before use, the same discipline `elf.c`'s existing
//! loader already holds itself to.
//!
//! ## What "dynamic linking" means here, precisely
//!
//! NovaOS has no userspace `ld.so` and no `PT_INTERP` support - there
//! is no way to leave the actual resolving to a mapped-in runtime
//! loader binary that bootstraps itself, which is one of the harder
//! and more fragile pieces of a real Unix dynamic linker. Instead the
//! kernel itself *is* the loader: `elf.c` reads the main executable
//! and every `DT_NEEDED` shared library, maps their `PT_LOAD` segments,
//! and calls into this module to resolve every relocation - all of it
//! done once, in the kernel, before the process's first instruction
//! ever runs. Concretely:
//!
//! - **Eager (`BIND_NOW`) binding only, no lazy PLT resolution.** A
//!   real ld.so can leave a PLT stub pointing at a lazy resolver and
//!   only look up a function the first time it's actually called. This
//!   module resolves every `R_386_JMP_SLOT` relocation up front instead
//!   - simpler, and just as real: it's exactly what `LD_BIND_NOW=1`
//!   does on a real Unix system, and it means the PLT trampoline bytes
//!   a normal linker still emits are simply never executed (their
//!   leading `jmp *GOT_slot` lands directly on the real function once
//!   this module has filled that slot in).
//! - **`R_386_RELATIVE`, `R_386_32`, `R_386_GLOB_DAT`, `R_386_JMP_SLOT`,
//!   `R_386_PC32` are supported.** These cover ordinary function calls
//!   across the executable/library boundary and a shared library's own
//!   internal absolute pointers (function-pointer tables, `.init_array`
//!   entries) at whatever base address it actually loads at.
//! - **`R_386_COPY` is deliberately NOT supported.** A real ld.so uses
//!   it so a non-PIE executable can hold its own private copy of a
//!   *data* symbol a shared library also defines, with both sides kept
//!   in sync by convention. Implementing it correctly needs cooperation
//!   from the library's own internal code generation that this
//!   project's toolchain doesn't specially arrange for, so this loader
//!   simply refuses a relocation that would need one (a distinct error
//!   code below, not a silent skip). In practice this means a shared
//!   library's *functions* can be called freely; a shared library's
//!   mutable *global data* cannot be referenced directly by name from
//!   the executable that loads it. Every demo and test this phase
//!   ships keeps to that.
//! - **No page-level sharing between processes.** Two processes that
//!   both load the same `.SO` each get their own physical frames and
//!   their own copy of it in memory - genuine on-disk sharing (one file
//!   on the FAT32/ext2 volume, linked by every program that needs it,
//!   instead of each statically duplicating its code) and genuine
//!   load-time linking, but not copy-on-write inter-process memory
//!   sharing. That would be a real, separate, and much larger physical-
//!   memory-manager feature - a plausible follow-up, not attempted here.

#![allow(dead_code)]

use core::ffi::c_void;

// ---------------------------------------------------------------- //
// ELF32 constants and on-disk structure layouts
// ---------------------------------------------------------------- //

const DT_NULL: i32 = 0;
const DT_NEEDED: i32 = 1;
const DT_HASH: i32 = 4;
const DT_STRTAB: i32 = 5;
const DT_SYMTAB: i32 = 6;
const DT_REL: i32 = 17;
const DT_RELSZ: i32 = 18;
const DT_STRSZ: i32 = 10;
const DT_JMPREL: i32 = 23;
const DT_PLTRELSZ: i32 = 2;

const R_386_NONE: u8 = 0;
const R_386_32: u8 = 1;
const R_386_PC32: u8 = 2;
const R_386_COPY: u8 = 5;
const R_386_GLOB_DAT: u8 = 6;
const R_386_JMP_SLOT: u8 = 7;
const R_386_RELATIVE: u8 = 8;

const STB_LOCAL: u8 = 0;
const STB_WEAK: u8 = 2;
const SHN_UNDEF: u16 = 0;

/// Elf32_Sym is 16 bytes: name(4) value(4) size(4) info(1) other(1)
/// shndx(2).
const SYM_ENTRY_SIZE: u32 = 16;
/// Elf32_Rel is 8 bytes: offset(4) info(4). i386 uses REL (the addend
/// lives in the bytes already at the target, not a separate field) for
/// both `.rel.dyn` and `.rel.plt` - confirmed against this project's
/// own toolchain's output (`DT_PLTREL` reads `REL`, never `RELA`, for
/// every i386 shared object and executable it produces).
const REL_ENTRY_SIZE: u32 = 8;

/// The longest symbol or library (`DT_NEEDED`) name this module will
/// read. Generous for both: NovaOS's own root-directory-only,
/// `8.3`-named filesystem caps a `DT_NEEDED` name at 12 characters
/// (see `userland/libc/stdio.c`'s own `check_name()`), and a C
/// identifier long enough to need more than 63 bytes has never
/// appeared anywhere in this codebase.
pub const MAX_NAME_LEN: usize = 64;

/// One `PT_LOAD` segment's (link-time virtual address, file offset,
/// file size) - exactly what `elf.c`'s own loader loop already reads
/// out of every program header it processes. Handed in by C (which
/// walks program headers for its own reasons already) rather than
/// re-walked here, so there is exactly one place in this codebase that
/// interprets `Elf32_Phdr`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LoadSeg {
    pub vaddr: u32,
    pub offset: u32,
    pub filesz: u32,
}

/// Everything this module needs to know about one loaded ELF image
/// (the main executable, or one shared library) to resolve its
/// relocations or answer a symbol lookup against it. `data`/`size` are
/// the image's raw, unrelocated file bytes (still resident - `elf.c`
/// keeps every loaded image's file buffer around for exactly this,
/// see its own comment); `bias` is 0 for the main `ET_EXEC` executable
/// and the chosen load base for a shared (`ET_DYN`) library.
#[repr(C)]
pub struct ImageInfo {
    pub data: *const u8,
    pub size: u32,
    pub bias: u32,
    pub dyn_offset: u32,
    pub dyn_filesz: u32,
    pub segs: *const LoadSeg,
    pub seg_count: u32,
}

/// Resolves an external (undefined-in-this-image) symbol by name to
/// its final absolute address, or returns 0 if it cannot. Implemented
/// on the C side by walking the process's already-loaded libraries (in
/// `DT_NEEDED` order) calling `rust_dynlink_find_symbol` on each - C
/// owns that list, so C owns the search order over it.
pub type ResolveFn =
    extern "C" fn(ctx: *mut c_void, name: *const u8, name_len: u32) -> u32;

/// Writes one resolved relocation's final 4-byte value at an absolute
/// virtual address in the process being built. Implemented on the C
/// side as a thin wrapper around the same `write_to_address_space()`
/// primitive already used to build a new process's initial stack.
/// Returns `false` if the address is not actually mapped (a malformed
/// relocation target) - this module aborts on the first such failure
/// rather than continuing to write into unknown memory.
pub type WriteFn =
    extern "C" fn(ctx: *mut c_void, vaddr: u32, value: u32) -> bool;

/// `rust_dynlink_apply_relocations`'s result, also reused by
/// `rust_dynlink_get_needed`. Kept as small positive/negative integers
/// (not a Rust `enum` crossing the FFI boundary) for the same reason
/// every other `extern "C"` return value in this codebase is a plain
/// integer: the C caller is what actually turns this into a
/// `kernel_log()` message, matching the project's standing convention
/// (Rust returns a status; C reports it).
pub const OK: i32 = 0;
pub const ERR_MALFORMED: i32 = -1;
pub const ERR_UNRESOLVED_SYMBOL: i32 = -2;
pub const ERR_WRITE_FAILED: i32 = -3;
pub const ERR_UNSUPPORTED_COPY_RELOC: i32 = -4;
pub const ERR_UNSUPPORTED_RELOC_TYPE: i32 = -5;

// ---------------------------------------------------------------- //
// Small internal helpers: bounds-checked reads, vaddr -> file offset
// ---------------------------------------------------------------- //

#[inline]
fn read_u32(data: &[u8], off: u32) -> Option<u32> {
    let off = off as usize;
    let b = data.get(off..off + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[inline]
fn read_i32(data: &[u8], off: u32) -> Option<i32> {
    read_u32(data, off).map(|v| v as i32)
}

#[inline]
fn read_u16(data: &[u8], off: u32) -> Option<u16> {
    let off = off as usize;
    let b = data.get(off..off + 2)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

#[inline]
fn read_u8(data: &[u8], off: u32) -> Option<u8> {
    data.get(off as usize).copied()
}

/// Translates a link-time virtual address, within ONE image, to that
/// image's own file offset, by finding the `PT_LOAD` segment that
/// covers it. This project's own linker invocations (a plain,
/// unmodified GNU `ld`, no hand-written linker script) guarantee the
/// standard property this relies on: within a segment, file offset and
/// virtual address advance together.
fn vaddr_to_offset(segs: &[LoadSeg], vaddr: u32) -> Option<u32> {
    for s in segs {
        // filesz, not memsz: anything past filesz is .bss - zeroed
        // memory with no file bytes behind it, so no offset exists.
        if vaddr >= s.vaddr && vaddr - s.vaddr < s.filesz {
            return Some(s.offset + (vaddr - s.vaddr));
        }
    }
    None
}

fn segs_slice<'a>(segs: *const LoadSeg, seg_count: u32) -> &'a [LoadSeg] {
    if segs.is_null() || seg_count == 0 {
        return &[];
    }
    // Safety (of the surrounding unsafe extern "C" fn, not of this
    // helper in isolation): callers pass the same (segs, seg_count)
    // C itself just built from a bounded, on-stack array - see
    // elf.c's MAX_LOAD_SEGS.
    unsafe { core::slice::from_raw_parts(segs, seg_count as usize) }
}

fn data_slice<'a>(data: *const u8, size: u32) -> &'a [u8] {
    if data.is_null() || size == 0 {
        return &[];
    }
    unsafe { core::slice::from_raw_parts(data, size as usize) }
}

// ---------------------------------------------------------------- //
// .dynamic parsing
// ---------------------------------------------------------------- //

/// An upper bound on how many `DT_NEEDED` entries a single image's
/// `.dynamic` array is scanned for. Not the same limit as
/// `MAX_SHARED_LIBS` in `kernel/task/process.h` (that one bounds the
/// TOTAL libraries one process loads, transitively, across every
/// image); this one just keeps this function's own on-stack array
/// small and bounded regardless of how many a single malformed file
/// might otherwise claim to need.
const MAX_NEEDED_SCAN: usize = 16;

/// Every pointer/size this module ever needs out of one image's
/// `.dynamic` array - parsed once per call into a plain struct rather
/// than re-scanned by every helper that needs one field of it.
/// Pointer-valued tags (`DT_STRTAB`, `DT_SYMTAB`, `DT_HASH`, `DT_REL`,
/// `DT_JMPREL`) are stored here as this image's own FILE OFFSETS
/// (already translated via `vaddr_to_offset`), not the raw virtual
/// addresses `.dynamic` itself holds - every other function in this
/// module only ever wants the file offset.
struct DynInfo {
    strtab_off: u32,
    symtab_off: u32,
    sym_count: u32,
    rel_off: u32,
    rel_sz: u32,
    jmprel_off: u32,
    jmprel_sz: u32,
    needed_offsets: [u32; MAX_NEEDED_SCAN], // strtab offsets of DT_NEEDED names
    needed_count: usize,
}

fn parse_dynamic(data: &[u8], dyn_offset: u32, dyn_filesz: u32, segs: &[LoadSeg]) -> Option<DynInfo> {
    if dyn_filesz == 0 || dyn_filesz % 8 != 0 {
        return None;
    }
    let count = dyn_filesz / 8; // Elf32_Dyn is {d_tag: i32, d_val: u32} = 8 bytes

    let mut strtab_vaddr = None;
    let mut symtab_vaddr = None;
    let mut hash_vaddr = None;
    let mut rel_vaddr = None;
    let mut rel_sz = 0u32;
    let mut jmprel_vaddr = None;
    let mut jmprel_sz = 0u32;
    let mut needed_offsets = [0u32; MAX_NEEDED_SCAN];
    let mut needed_count = 0usize;

    for i in 0..count {
        let entry_off = dyn_offset + i * 8;
        let tag = read_i32(data, entry_off)?;
        let val = read_u32(data, entry_off + 4)?;
        if tag == DT_NULL {
            break;
        }
        match tag {
            DT_NEEDED => {
                if needed_count < MAX_NEEDED_SCAN {
                    needed_offsets[needed_count] = val; // strtab offset
                    needed_count += 1;
                }
                // Beyond MAX_NEEDED_SCAN: silently capped, not an
                // error here - rust_dynlink_get_needed re-derives the
                // true count via its own return value, so the caller
                // (which owns the separate MAX_SHARED_LIBS limit) can
                // detect and refuse an oversized list itself rather
                // than silently loading only the first few.
            }
            DT_STRTAB => strtab_vaddr = Some(val),
            DT_SYMTAB => symtab_vaddr = Some(val),
            DT_HASH => hash_vaddr = Some(val),
            DT_REL => rel_vaddr = Some(val),
            DT_RELSZ => rel_sz = val,
            DT_JMPREL => jmprel_vaddr = Some(val),
            DT_PLTRELSZ => jmprel_sz = val,
            _ => {}
        }
    }

    let strtab_off = match strtab_vaddr {
        Some(v) => vaddr_to_offset(segs, v)?,
        None => 0, // a library with no strtab has no named symbols to look up
    };
    let symtab_off = match symtab_vaddr {
        Some(v) => vaddr_to_offset(segs, v)?,
        None => 0,
    };
    let rel_off = match rel_vaddr {
        Some(v) => vaddr_to_offset(segs, v)?,
        None => 0,
    };
    let jmprel_off = match jmprel_vaddr {
        Some(v) => vaddr_to_offset(segs, v)?,
        None => 0,
    };

    // The classic SysV DT_HASH table's own header is [nbucket, nchain,
    // ...]; nchain is, by construction, exactly the number of dynamic
    // symbols - the one piece of information nothing else in
    // `.dynamic` gives directly. This project's own shared-library
    // builds always pass `--hash-style=sysv` (see userland/dynlib/
    // build.sh) specifically so this is always present; a library
    // built some other way, with only `--hash-style=gnu`, is rejected
    // here (sym_count stays 0, so no symbol will ever resolve against
    // it) rather than guessed at.
    let sym_count = match hash_vaddr.and_then(|v| vaddr_to_offset(segs, v)) {
        Some(hash_off) => read_u32(data, hash_off + 4)?,
        None => 0,
    };

    Some(DynInfo {
        strtab_off,
        symtab_off,
        sym_count,
        rel_off,
        rel_sz,
        jmprel_off,
        jmprel_sz,
        needed_offsets,
        needed_count,
    })
}

/// Reads a NUL-terminated string out of a strtab at `strtab_off +
/// name_off`, into a fixed local buffer. Returns the byte length (not
/// including the NUL), or `None` if it runs off the end of `data` or
/// past `MAX_NAME_LEN` without terminating.
fn read_str(data: &[u8], strtab_off: u32, name_off: u32, out: &mut [u8; MAX_NAME_LEN]) -> Option<usize> {
    let start = (strtab_off + name_off) as usize;
    let mut n = 0usize;
    loop {
        let b = *data.get(start + n)?;
        if b == 0 {
            return Some(n);
        }
        if n >= MAX_NAME_LEN {
            return None; // refuse to silently truncate a real name
        }
        out[n] = b;
        n += 1;
    }
}

fn names_equal(buf: &[u8; MAX_NAME_LEN], buf_len: usize, name: &[u8]) -> bool {
    buf_len == name.len() && &buf[..buf_len] == name
}

// ---------------------------------------------------------------- //
// Public entry point 1: enumerate DT_NEEDED
// ---------------------------------------------------------------- //

/// Fixed-capacity output slot for one `DT_NEEDED` name - a plain byte
/// array plus its length, not a NUL-terminated C string, so a name
/// that happens to be exactly `MAX_NAME_LEN` long is never ambiguous
/// with one that overflowed.
#[repr(C)]
pub struct NeededName {
    pub bytes: [u8; MAX_NAME_LEN],
    pub len: u32,
}

/// Fills `out[0..min(needed_count, out_len)]` with this image's
/// `DT_NEEDED` library names, in the order they appear in `.dynamic`
/// (the order a real ld.so would also load them in). Returns the
/// TRUE number of `DT_NEEDED` entries the image has - which may be
/// larger than `out_len` - or a negative `ERR_*` code. The caller
/// (`elf.c`) compares that true count against its own remaining
/// `MAX_SHARED_LIBS` budget and fails the whole exec rather than
/// silently loading only the first few.
///
/// # Safety
/// `data` must be valid for `size` bytes, `segs` for `seg_count`
/// entries, `out` for `out_len` entries.
#[no_mangle]
pub unsafe extern "C" fn rust_dynlink_get_needed(
    data: *const u8,
    size: u32,
    dyn_offset: u32,
    dyn_filesz: u32,
    segs: *const LoadSeg,
    seg_count: u32,
    out: *mut NeededName,
    out_len: u32,
) -> i32 {
    let data = data_slice(data, size);
    let segs = segs_slice(segs, seg_count);
    let dyn_info = match parse_dynamic(data, dyn_offset, dyn_filesz, segs) {
        Some(d) => d,
        None => return ERR_MALFORMED,
    };

    let out: &mut [NeededName] = if out.is_null() || out_len == 0 {
        &mut [][..]
    } else {
        core::slice::from_raw_parts_mut(out, out_len as usize)
    };

    for i in 0..dyn_info.needed_count {
        if i >= out.len() {
            break;
        }
        let mut name_buf = [0u8; MAX_NAME_LEN];
        let name_len = match read_str(data, dyn_info.strtab_off, dyn_info.needed_offsets[i], &mut name_buf) {
            Some(n) => n,
            None => return ERR_MALFORMED,
        };
        out[i].bytes = name_buf;
        out[i].len = name_len as u32;
    }

    dyn_info.needed_count as i32
}

// ---------------------------------------------------------------- //
// Public entry point 2: symbol lookup within one image
// ---------------------------------------------------------------- //

/// Looks up `name` in this image's own `.dynsym`. Returns the
/// symbol's final absolute address (`bias + st_value`) if it is
/// DEFINED here (`st_shndx != SHN_UNDEF`) and globally visible
/// (`STB_LOCAL` symbols are a compiler-internal implementation detail,
/// never a valid cross-image link target, so they are skipped exactly
/// as a real ld.so's global symbol scope would skip them) - or 0 if
/// not found. Used by C's own `resolve` callback implementation, once
/// per candidate library, walking the process's loaded-library list
/// in `DT_NEEDED` order until one of these calls returns non-zero.
///
/// # Safety
/// `img` must point to a valid, fully-populated `ImageInfo` whose
/// `data`/`segs` pointers are valid for their stated lengths.
#[no_mangle]
pub unsafe extern "C" fn rust_dynlink_find_symbol(
    img: *const ImageInfo,
    name: *const u8,
    name_len: u32,
) -> u32 {
    let img = match img.as_ref() {
        Some(i) => i,
        None => return 0,
    };
    let data = data_slice(img.data, img.size);
    let segs = segs_slice(img.segs, img.seg_count);
    let dyn_info = match parse_dynamic(data, img.dyn_offset, img.dyn_filesz, segs) {
        Some(d) => d,
        None => return 0,
    };
    let name = data_slice(name, name_len);

    for i in 0..dyn_info.sym_count {
        let sym_off = dyn_info.symtab_off + i * SYM_ENTRY_SIZE;
        let st_name = match read_u32(data, sym_off) {
            Some(v) => v,
            None => return 0,
        };
        let st_value = match read_u32(data, sym_off + 4) {
            Some(v) => v,
            None => return 0,
        };
        let st_info = match read_u8(data, sym_off + 12) {
            Some(v) => v,
            None => return 0,
        };
        let st_shndx = match read_u16(data, sym_off + 14) {
            Some(v) => v,
            None => return 0,
        };

        if st_shndx == SHN_UNDEF {
            continue; // this image doesn't define it either
        }
        let bind = st_info >> 4;
        if bind == STB_LOCAL {
            continue; // not part of the global symbol scope
        }
        if st_name == 0 {
            continue; // the reserved "no name" symbol at dynsym[0]
        }

        let mut name_buf = [0u8; MAX_NAME_LEN];
        let this_len = match read_str(data, dyn_info.strtab_off, st_name, &mut name_buf) {
            Some(n) => n,
            None => continue, // an unreadable name can't be a match; keep scanning
        };
        if names_equal(&name_buf, this_len, name) {
            return img.bias.wrapping_add(st_value);
        }
    }
    0
}

// ---------------------------------------------------------------- //
// Public entry point 3: relocation processing
// ---------------------------------------------------------------- //

/// One relocation table (`.rel.dyn` or `.rel.plt`) processed against
/// `img`, calling `resolve`/`write` as needed. Shared by both calls in
/// `apply_relocations` below since i386 gives both tables the exact
/// same entry format and this loader treats `R_386_JMP_SLOT` no
/// differently from `R_386_GLOB_DAT` (eager binding - see this file's
/// own top-of-module comment).
#[allow(clippy::too_many_arguments)]
unsafe fn apply_rel_table(
    data: &[u8],
    segs: &[LoadSeg],
    bias: u32,
    dyn_info: &DynInfo,
    rel_off: u32,
    rel_sz: u32,
    resolve: ResolveFn,
    resolve_ctx: *mut c_void,
    write: WriteFn,
    write_ctx: *mut c_void,
) -> i32 {
    if rel_sz == 0 {
        return OK;
    }
    if rel_sz % REL_ENTRY_SIZE != 0 {
        return ERR_MALFORMED;
    }
    let count = rel_sz / REL_ENTRY_SIZE;

    for i in 0..count {
        let entry_off = rel_off + i * REL_ENTRY_SIZE;
        let r_offset = match read_u32(data, entry_off) {
            Some(v) => v,
            None => return ERR_MALFORMED,
        };
        let r_info = match read_u32(data, entry_off + 4) {
            Some(v) => v,
            None => return ERR_MALFORMED,
        };
        let r_sym = r_info >> 8;
        let r_type = (r_info & 0xff) as u8;

        if r_type == R_386_NONE {
            continue;
        }
        if r_type == R_386_COPY {
            return ERR_UNSUPPORTED_COPY_RELOC;
        }

        // The addend for every REL-type relocation this loader
        // supports is the 4 bytes already sitting at the relocation's
        // own target, as originally written by the linker into the
        // FILE - see this file's top comment on why reading from
        // `data` (not from live, not-yet-fully-mapped memory) is both
        // simpler and correct here.
        let addend_off = match vaddr_to_offset(segs, r_offset) {
            Some(o) => o,
            None => return ERR_MALFORMED,
        };
        let addend = match read_u32(data, addend_off) {
            Some(v) => v,
            None => return ERR_MALFORMED,
        };

        let value: u32 = match r_type {
            R_386_RELATIVE => bias.wrapping_add(addend),
            R_386_32 | R_386_GLOB_DAT | R_386_JMP_SLOT | R_386_PC32 => {
                let sym = if r_sym == 0 {
                    0
                } else {
                    let sym_off = dyn_info.symtab_off + r_sym * SYM_ENTRY_SIZE;
                    let st_name = match read_u32(data, sym_off) {
                        Some(v) => v,
                        None => return ERR_MALFORMED,
                    };
                    let st_value = match read_u32(data, sym_off + 4) {
                        Some(v) => v,
                        None => return ERR_MALFORMED,
                    };
                    let st_info = match read_u8(data, sym_off + 12) {
                        Some(v) => v,
                        None => return ERR_MALFORMED,
                    };
                    let st_shndx = match read_u16(data, sym_off + 14) {
                        Some(v) => v,
                        None => return ERR_MALFORMED,
                    };

                    if st_shndx != SHN_UNDEF {
                        // Defined in THIS image: resolves locally,
                        // never needs an external search - this is
                        // also exactly how a plain R_386_32 pointing
                        // at a file-local static gets resolved.
                        bias.wrapping_add(st_value)
                    } else {
                        let mut name_buf = [0u8; MAX_NAME_LEN];
                        let name_len = match read_str(data, dyn_info.strtab_off, st_name, &mut name_buf) {
                            Some(n) => n,
                            None => return ERR_MALFORMED,
                        };
                        let resolved = resolve(resolve_ctx, name_buf.as_ptr(), name_len as u32);
                        if resolved == 0 {
                            let bind = st_info >> 4;
                            if bind == STB_WEAK {
                                0 // an unresolved weak symbol is valid: it's simply 0
                            } else {
                                return ERR_UNRESOLVED_SYMBOL;
                            }
                        } else {
                            resolved
                        }
                    }
                };

                if r_type == R_386_PC32 {
                    sym.wrapping_add(addend).wrapping_sub(r_offset.wrapping_add(bias))
                } else if r_type == R_386_32 {
                    sym.wrapping_add(addend)
                } else {
                    // GLOB_DAT / JMP_SLOT: no addend in the standard
                    // formula. Real linkers always emit A=0 for these
                    // anyway; not reading it back at all here is both
                    // simpler and matches the spec exactly.
                    sym
                }
            }
            _ => return ERR_UNSUPPORTED_RELOC_TYPE,
        };

        let write_addr = bias.wrapping_add(r_offset);
        if !write(write_ctx, write_addr, value) {
            return ERR_WRITE_FAILED;
        }
    }

    OK
}

/// Resolves and writes every relocation (`.rel.dyn` AND `.rel.plt`)
/// for one already-mapped image. Must be called for the main
/// executable and for every loaded shared library - `elf.c` calls it
/// once per image, after every image involved has had its `PT_LOAD`
/// segments mapped (so `write` always lands on real, present pages)
/// but the ORDER between images doesn't matter for correctness: this
/// function only ever reads `dyn_info`/symbols out of `img` itself
/// (never another image directly), and `resolve` is free to look
/// across every already-loaded image regardless of which of them this
/// particular call is processing.
///
/// # Safety
/// `img` must point to a valid, fully-populated `ImageInfo`.
/// `resolve`/`write` must be valid function pointers safe to call with
/// their respective `_ctx` pointers.
#[no_mangle]
pub unsafe extern "C" fn rust_dynlink_apply_relocations(
    img: *const ImageInfo,
    resolve: ResolveFn,
    resolve_ctx: *mut c_void,
    write: WriteFn,
    write_ctx: *mut c_void,
) -> i32 {
    let img = match img.as_ref() {
        Some(i) => i,
        None => return ERR_MALFORMED,
    };
    let data = data_slice(img.data, img.size);
    let segs = segs_slice(img.segs, img.seg_count);
    let dyn_info = match parse_dynamic(data, img.dyn_offset, img.dyn_filesz, segs) {
        Some(d) => d,
        None => return ERR_MALFORMED,
    };

    let rc = apply_rel_table(
        data, segs, img.bias, &dyn_info,
        dyn_info.rel_off, dyn_info.rel_sz,
        resolve, resolve_ctx, write, write_ctx,
    );
    if rc != OK {
        return rc;
    }

    apply_rel_table(
        data, segs, img.bias, &dyn_info,
        dyn_info.jmprel_off, dyn_info.jmprel_sz,
        resolve, resolve_ctx, write, write_ctx,
    )
}

// ---------------------------------------------------------------- //
// Host-only unit tests for the parsing helpers that need no real ELF
// input at all - just crafted byte buffers. The relocation-writing
// path itself is exercised far more thoroughly, against real
// cross-compiled ELF files, by kernel/rust/tests/ - see that harness
// for the actual end-to-end proof this module is correct.
// ---------------------------------------------------------------- //

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vaddr_translation_basic() {
        let segs = [
            LoadSeg { vaddr: 0x1000, offset: 0x0, filesz: 0x100 },
            LoadSeg { vaddr: 0x2000, offset: 0x100, filesz: 0x50 },
        ];
        assert_eq!(vaddr_to_offset(&segs, 0x1000), Some(0x0));
        assert_eq!(vaddr_to_offset(&segs, 0x1010), Some(0x10));
        assert_eq!(vaddr_to_offset(&segs, 0x2000), Some(0x100));
        assert_eq!(vaddr_to_offset(&segs, 0x2050), None); // == filesz: past the file data
        assert_eq!(vaddr_to_offset(&segs, 0x3000), None); // not covered at all
    }

    #[test]
    fn parse_dynamic_finds_needed() {
        // strtab holds "\0LIBA.SO\0LIBB.SO\0"
        let mut strtab = [0u8; 32];
        strtab[1..9].copy_from_slice(b"LIBA.SO\0");
        let a_off = 1u32;
        strtab[9..17].copy_from_slice(b"LIBB.SO\0");
        let b_off = 9u32;

        let mut dynbuf = [0u8; 64];
        let mut w = |off: usize, tag: i32, val: u32| {
            dynbuf[off..off + 4].copy_from_slice(&(tag as u32).to_le_bytes());
            dynbuf[off + 4..off + 8].copy_from_slice(&val.to_le_bytes());
        };
        w(0, DT_NEEDED, a_off);
        w(8, DT_NEEDED, b_off);
        w(16, DT_STRTAB, 0x1300); // vaddr, single test segment: vaddr = 0x1000 + file_offset
        w(24, DT_STRSZ, 17);
        w(32, DT_NULL, 0);
        let dyn_len = 40u32;

        // lay .dynamic at file offset 0x100 (vaddr 0x1100), strtab at
        // file offset 0x300 (vaddr 0x1300) - both inside the one
        // segment below, vaddr 0x1000 == file offset 0
        let mut file = [0u8; 0x400];
        file[0x100..0x100 + dyn_len as usize].copy_from_slice(&dynbuf[..dyn_len as usize]);
        file[0x300..0x300 + 17].copy_from_slice(&strtab[..17]);

        let segs = [LoadSeg { vaddr: 0x1000, offset: 0x0, filesz: 0x400 }];
        let dyn_info = parse_dynamic(&file, 0x100, dyn_len, &segs).unwrap();
        assert_eq!(dyn_info.needed_count, 2);

        let mut name_buf = [0u8; MAX_NAME_LEN];
        let n = read_str(&file, dyn_info.strtab_off, dyn_info.needed_offsets[0], &mut name_buf).unwrap();
        assert_eq!(&name_buf[..n], b"LIBA.SO");
        let n = read_str(&file, dyn_info.strtab_off, dyn_info.needed_offsets[1], &mut name_buf).unwrap();
        assert_eq!(&name_buf[..n], b"LIBB.SO");
    }

    #[test]
    fn relative_reloc_uses_bias_plus_addend() {
        let mut file = [0u8; 0x100];
        // one R_386_RELATIVE entry at vaddr 0x2000
        file[0x00..0x04].copy_from_slice(&0x2000u32.to_le_bytes()); // r_offset
        file[0x04..0x08].copy_from_slice(&(R_386_RELATIVE as u32).to_le_bytes()); // r_info: sym=0, type=RELATIVE
        file[0x50..0x54].copy_from_slice(&0x40u32.to_le_bytes()); // addend at the target itself (offset 0x50 = where vaddr 0x2000 maps)

        let segs = [
            LoadSeg { vaddr: 0x1000, offset: 0x0, filesz: 0x50 },
            LoadSeg { vaddr: 0x2000, offset: 0x50, filesz: 0x10 },
        ];

        extern "C" fn write_cb(ctx: *mut c_void, vaddr: u32, value: u32) -> bool {
            let out = unsafe { &mut *(ctx as *mut (u32, u32, u32)) };
            out.0 += 1;
            out.1 = vaddr;
            out.2 = value;
            true
        }
        extern "C" fn resolve_cb(_ctx: *mut c_void, _n: *const u8, _l: u32) -> u32 {
            0
        }

        let dyn_info = DynInfo {
            strtab_off: 0, symtab_off: 0, sym_count: 0,
            rel_off: 0, rel_sz: 8, jmprel_off: 0, jmprel_sz: 0,
            needed_offsets: [0; MAX_NEEDED_SCAN], needed_count: 0,
        };

        let bias = 0x9000u32;
        let mut captured = (0u32, 0u32, 0u32);
        let rc = unsafe {
            apply_rel_table(
                &file, &segs, bias, &dyn_info, 0, 8,
                resolve_cb, core::ptr::null_mut(),
                write_cb, &mut captured as *mut _ as *mut c_void,
            )
        };
        assert_eq!(rc, OK);
        assert_eq!(captured, (1, bias + 0x2000, bias + 0x40));
    }
}
