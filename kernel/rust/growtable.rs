//! kernel/rust/growtable.rs - Phase 75: a generic, reusable growable
//! table. Turns a `static process_t process_table[MAX_PROCESSES]`-
//! style fixed C array into one that grows on demand, without ever
//! invalidating a pointer to an element already handed out - the one
//! property that actually matters here, since this kernel hands out
//! raw `process_t*` pointers (`process_table_entry()`,
//! `current[cpu_index]`, every process_t* held mid-syscall) that
//! outlive the call that produced them, read from both CPUs
//! concurrently.
//!
//! ## Design: a chunk table, not a reallocating array
//!
//! An ordinary growable array (realloc a bigger block, copy the old
//! contents in, free the old block) MOVES every element already in
//! it - fatal here. Instead, elements live in fixed-size CHUNKS, each
//! one allocated exactly once (via the C heap allocator - see
//! `kmalloc`/`kfree` below) and never moved or freed again for the
//! rest of the kernel's life. Growing means allocating one MORE chunk
//! and recording its address; every existing chunk, and every pointer
//! into one, is completely undisturbed.
//!
//! The only thing that grows in the ordinary, reallocating sense is
//! the ARRAY OF CHUNK POINTERS itself - and since that only stores
//! addresses, not full elements, giving it a generous fixed capacity
//! (`MAX_CHUNKS`) up front costs almost nothing and needs no
//! reallocation logic at all.
//!
//! ## What this module knows, and what it doesn't
//!
//! This module has no idea what a `process_t` (or any other element
//! type) actually contains - every element is just `element_size`
//! opaque bytes, and every returned pointer is `*mut u8`. The C
//! caller casts it to whatever real struct type that particular table
//! holds, exactly as it always indexed a plain array of that type.
//! That split matches every other `kernel/rust/` module here: OS
//! integration (memory allocation) is a link-time call into C
//! (`kmalloc`/`kfree` - see `journal.rs`'s own `blockdev_*` `extern
//! "C"` block for the identical, established pattern); the logic
//! actually worth having in one well-tested place (chunk bookkeeping,
//! bounds checks, the stable-pointer guarantee) lives here.
//!
//! Multiple independent tables can exist at once (`rust_growtable_
//! create()` returns a small integer handle) - not because this
//! phase's own primary target, the process table, needs more than
//! one, but because "process table, etc." is genuinely this
//! project's own roadmap wording, and a component this generic costs
//! nothing extra to make properly reusable rather than hard-coded to
//! one specific caller.
//!
//! ## Concurrency
//!
//! This module does none of its own locking - by design, the same
//! choice every other `kernel/rust/` module makes. `rust_growtable_
//! ensure_capacity()` (the only function that mutates shared state -
//! adding a chunk) must only ever be called while the CALLER already
//! holds whatever lock guards that specific table's contents (for the
//! process table, the existing `process_table_lock` - see
//! `kernel/task/process.c`, which already held that same lock around
//! the scan this replaces). Read-only calls (`slot_ptr`, `capacity`)
//! need no lock of their own: the chunk-pointer array is append-only
//! and never shrinks or moves an existing entry, so a stale-but-not-
//! wrong read (an index whose chunk was allocated a moment after this
//! read began) is exactly the same benign race `process_table[]`'s
//! own unlocked reads already tolerated before this phase (see that
//! file's own comment on `process_table_lock`'s narrow scope).
//!
//! ## What this phase deliberately does NOT also fix
//!
//! A process's slot, once allocated, is never returned to the free
//! list even after that process has exited and been reaped -
//! `kernel/task/process.c`'s own pre-existing comment already named
//! this ("slots are never actually recycled today"). This module
//! makes the table grow instead of hitting a hard ceiling, which is
//! the actual, named ask; it does not additionally implement slot
//! recycling. That is a real, separate, and non-trivial question of
//! its own - `process_wait()`/`process_wait_nonblock()` are
//! deliberately safe to call more than once for the same pid, still
//! returning the same exit code every time (multiple call sites in
//! this codebase rely on exactly that), and immediately recycling a
//! slot the instant it's first reaped would break that guarantee for
//! any second caller. Fixing it properly needs its own design (most
//! plausibly: recycle only once nothing could plausibly still want
//! that pid's exit code, which is not a question this phase's own
//! scope - "does the table grow instead of hitting a wall" - actually
//! needs to answer). Documented here, not silently left unmentioned.

#![allow(dead_code)]

// ---------------------------------------------------------------- //
// The C heap allocator. Real declaration when compiled into the
// kernel; a host-side stand-in (std::alloc, real allocation - not a
// mock) when compiled as this file's own host test binary, so the
// exact same growth/bounds logic below runs and is checked either
// way, never a separately-maintained reimplementation of it. See
// journal.rs's own `extern "C" { fn blockdev_read_sectors(...); }`
// block for the identical established pattern this mirrors.
// ---------------------------------------------------------------- //

#[cfg(not(test))]
extern "C" {
    fn kmalloc(size: usize) -> *mut u8;
    #[allow(dead_code)]
    fn kfree(ptr: *mut u8);
}

#[cfg(test)]
unsafe fn kmalloc(size: usize) -> *mut u8 {
    use std::alloc::{alloc, Layout};
    alloc(Layout::from_size_align(size, 16).unwrap())
}
#[cfg(test)]
#[allow(dead_code)]
unsafe fn kfree(_ptr: *mut u8) {
    // Never actually called by this module (chunks live for the
    // kernel's whole lifetime) - see this file's own top comment.
    // Intentionally leaked in the host test build too, rather than
    // reconstructing the Layout kmalloc used (this module's own C
    // counterpart, kernel/arch/x86/mm/heap.c, tracks block sizes
    // itself precisely so its own real kfree() doesn't need one
    // either).
}

/// How many chunks a single table may grow to. 64 chunks of, say, 32
/// `process_t`-sized (roughly 170 bytes) elements each is 2048 total
/// slots ever handed out over the kernel's lifetime - a generous,
/// explicit, honest bound (matching this project's own standing
/// preference for a real, stated ceiling over an unstated "unlimited"
/// claim) rather than a tight fit against what the process table
/// specifically needs today. The chunk-pointer array this bounds is
/// tiny (`MAX_CHUNKS` raw pointers, 256 bytes on this 32-bit target)
/// regardless of how many of a table's chunks actually end up used.
const MAX_CHUNKS: usize = 64;

/// How many independent tables this module can track at once. Only
/// one is actually created by this phase (the process table); see
/// this file's own top comment on why more than one is supported
/// anyway. 16 is real headroom, not a tight fit - table handles are
/// never freed once created (matching every table's own chunks: both
/// live for the kernel's whole life), so this only ever needs to
/// exceed the total COUNT of distinct tables ever created, lifetime,
/// not any per-table size.
const MAX_TABLES: usize = 16;

struct Table {
    in_use: bool,
    element_size: usize,
    slots_per_chunk: usize,
    /// This table's own cap on chunk count, always <= MAX_CHUNKS -
    /// distinct from MAX_CHUNKS itself so two tables created with
    /// different limits don't share one global ceiling.
    max_chunks: usize,
    chunks: [*mut u8; MAX_CHUNKS],
    chunk_count: usize,
}

const EMPTY_TABLE: Table = Table {
    in_use: false,
    element_size: 0,
    slots_per_chunk: 0,
    max_chunks: 0,
    chunks: [core::ptr::null_mut(); MAX_CHUNKS],
    chunk_count: 0,
};

static mut TABLES: [Table; MAX_TABLES] = [EMPTY_TABLE; MAX_TABLES];

fn table_mut(handle: i32) -> Option<&'static mut Table> {
    if handle < 0 || handle as usize >= MAX_TABLES {
        return None;
    }
    // Safety: every mutating entry point requires the caller to hold
    // that table's own lock (see this file's top comment); every
    // read-only entry point tolerates the same benign race the
    // element data itself already did. One process, no threads
    // within it - the usual `static mut` caveat applies exactly as
    // it does to every other kernel/rust/ module's own global state.
    unsafe { TABLES.get_mut(handle as usize) }
}

/// Creates a new table of `element_size`-byte elements, `slots_per_
/// chunk` per chunk, growable up to `max_chunks` chunks (so
/// `slots_per_chunk * max_chunks` elements, total, ever). Returns a
/// small non-negative handle for the other functions below, or -1 if
/// every table slot is already in use, `max_chunks` exceeds
/// `MAX_CHUNKS`, or either size argument is 0.
///
/// Allocates no memory itself - a table starts with zero chunks and
/// zero capacity; the first `rust_growtable_ensure_capacity()` call
/// is what allocates its first chunk. This matters for the intended
/// call site (`process_init()`, which today does `memset(process_
/// table, 0, sizeof(process_table))` - Phase 75 keeps that same
/// "nothing to do yet" cost at boot, growing only once something is
/// actually created).
#[no_mangle]
pub extern "C" fn rust_growtable_create(
    element_size: u32,
    slots_per_chunk: u32,
    max_chunks: u32,
) -> i32 {
    if element_size == 0 || slots_per_chunk == 0 || max_chunks == 0 {
        return -1;
    }
    if max_chunks as usize > MAX_CHUNKS {
        return -1;
    }
    for i in 0..MAX_TABLES {
        if let Some(t) = table_mut(i as i32) {
            if !t.in_use {
                t.in_use = true;
                t.element_size = element_size as usize;
                t.slots_per_chunk = slots_per_chunk as usize;
                t.max_chunks = max_chunks as usize;
                t.chunk_count = 0;
                return i as i32;
            }
        }
    }
    -1
}

/// This table's current capacity (`chunk_count * slots_per_chunk`) -
/// how many valid indices `rust_growtable_slot_ptr()` will currently
/// accept. 0 for an invalid handle.
#[no_mangle]
pub extern "C" fn rust_growtable_capacity(handle: i32) -> u32 {
    match table_mut(handle) {
        Some(t) if t.in_use => (t.chunk_count * t.slots_per_chunk) as u32,
        _ => 0,
    }
}

/// Grows `handle`'s table, if needed, until its capacity is at least
/// `at_least` elements - allocating and zero-filling one more chunk
/// at a time (via `kmalloc`) until it is. Zero-filling a freshly
/// allocated chunk matters: it reproduces the one guarantee a plain
/// C `static process_t process_table[MAX_PROCESSES]` always gave for
/// free (BSS is zero-initialized), which every existing UNUSED-state
/// check in `kernel/task/process.c` depends on for a slot it has
/// never touched before.
///
/// MUST be called only while the caller holds whatever lock guards
/// this table (see this file's own top comment) - this is the one
/// function in this module that mutates shared state.
///
/// Returns `true` once capacity is at least `at_least` (including
/// trivially, if it already was); `false` if `handle` is invalid, if
/// growing further would exceed this table's own `max_chunks`, or if
/// `kmalloc` itself fails (out of kernel heap memory) - in which case
/// whatever chunks were successfully added before the failure are
/// kept (a partial grow is still real, useful progress, not
/// discarded), and the caller sees exactly the same "allocation
/// failed" outcome `allocate_slot()` already reports today when the
/// table is genuinely full.
///
/// # Safety
/// Calls `kmalloc`, an external C function; sound exactly when
/// `kmalloc` itself is (a simple heap allocator - see `kernel/arch/
/// x86/mm/heap.c`).
#[no_mangle]
pub unsafe extern "C" fn rust_growtable_ensure_capacity(handle: i32, at_least: u32) -> bool {
    let t = match table_mut(handle) {
        Some(t) if t.in_use => t,
        _ => return false,
    };
    let at_least = at_least as usize;

    while t.chunk_count * t.slots_per_chunk < at_least {
        if t.chunk_count >= t.max_chunks {
            return false;
        }
        let bytes = t.element_size * t.slots_per_chunk;
        let mem = kmalloc(bytes);
        if mem.is_null() {
            return false;
        }
        core::ptr::write_bytes(mem, 0, bytes);
        t.chunks[t.chunk_count] = mem;
        t.chunk_count += 1;
    }
    true
}

/// A stable pointer to element `index` of `handle`'s table -
/// `element_size` bytes the C caller casts to its real element type,
/// exactly as `&process_table[index]` always did. This pointer stays
/// valid for the rest of the kernel's life: growing a table never
/// moves or frees an existing chunk (see this file's own top
/// comment). Returns null if `handle` is invalid or `index` is
/// outside the table's current capacity (`rust_growtable_capacity()`)
/// - the caller must `rust_growtable_ensure_capacity()` first, the
/// same way `allocate_slot()` grows before claiming a new slot.
#[no_mangle]
pub extern "C" fn rust_growtable_slot_ptr(handle: i32, index: u32) -> *mut u8 {
    let t = match table_mut(handle) {
        Some(t) if t.in_use => t,
        _ => return core::ptr::null_mut(),
    };
    let index = index as usize;
    let capacity = t.chunk_count * t.slots_per_chunk;
    if index >= capacity {
        return core::ptr::null_mut();
    }
    let chunk_index = index / t.slots_per_chunk;
    let offset_in_chunk = index % t.slots_per_chunk;
    let chunk = t.chunks[chunk_index];
    // Safety: chunk_index < t.chunk_count (from the capacity check
    // above), so `chunk` was set by a real, successful kmalloc() in
    // rust_growtable_ensure_capacity() and never freed since.
    unsafe { chunk.add(offset_in_chunk * t.element_size) }
}

// ---------------------------------------------------------------- //
// Host-only unit tests. No kernel, no QEMU - real host allocation
// (see the `kmalloc` stand-in above) exercising the actual growth/
// bounds logic above, not a separate reimplementation of it.
// ---------------------------------------------------------------- //

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Debug)]
    struct TestElem {
        a: u32,
        b: u32,
        c: u32,
    }

    unsafe fn slot<'a>(h: i32, i: u32) -> &'a mut TestElem {
        let p = rust_growtable_slot_ptr(h, i);
        assert!(!p.is_null(), "slot {} should be valid", i);
        &mut *(p as *mut TestElem)
    }

    #[test]
    fn create_rejects_bad_args() {
        assert_eq!(rust_growtable_create(0, 4, 4), -1);
        assert_eq!(rust_growtable_create(12, 0, 4), -1);
        assert_eq!(rust_growtable_create(12, 4, 0), -1);
        assert_eq!(rust_growtable_create(12, 4, (MAX_CHUNKS + 1) as u32), -1);
    }

    #[test]
    fn starts_at_zero_capacity_grows_in_chunks() {
        let h = rust_growtable_create(12, 4, 8);
        assert!(h >= 0);
        assert_eq!(rust_growtable_capacity(h), 0);
        assert!(rust_growtable_slot_ptr(h, 0).is_null());

        unsafe {
            assert!(rust_growtable_ensure_capacity(h, 1));
        }
        assert_eq!(rust_growtable_capacity(h), 4); // rounds up to one whole chunk
        assert!(!rust_growtable_slot_ptr(h, 0).is_null());
        assert!(!rust_growtable_slot_ptr(h, 3).is_null());
        assert!(rust_growtable_slot_ptr(h, 4).is_null()); // one past current capacity

        unsafe {
            assert!(rust_growtable_ensure_capacity(h, 5));
        }
        assert_eq!(rust_growtable_capacity(h), 8); // grew by exactly one more chunk
        assert!(!rust_growtable_slot_ptr(h, 7).is_null());
        assert!(rust_growtable_slot_ptr(h, 8).is_null());
    }

    #[test]
    fn new_chunk_is_zeroed() {
        let h = rust_growtable_create(12, 4, 4);
        unsafe {
            assert!(rust_growtable_ensure_capacity(h, 4));
            for i in 0..4 {
                assert_eq!(*slot(h, i), TestElem { a: 0, b: 0, c: 0 });
            }
        }
    }

    #[test]
    fn existing_pointers_survive_growth() {
        // The entire point of this module: growing must never move an
        // element already handed out. Write a distinct pattern into
        // every slot of the first chunk, force several more chunks to
        // be allocated, then confirm the first chunk's own bytes -
        // and the pointers into it - are completely untouched.
        let h = rust_growtable_create(12, 4, 16);
        let mut first_chunk_ptrs = [core::ptr::null_mut::<TestElem>(); 4];
        unsafe {
            assert!(rust_growtable_ensure_capacity(h, 4));
            for i in 0..4u32 {
                let e = slot(h, i);
                *e = TestElem { a: i, b: i * 10, c: i * 100 };
                first_chunk_ptrs[i as usize] = e as *mut TestElem;
            }

            // Grow several more times, well past the first chunk.
            assert!(rust_growtable_ensure_capacity(h, 40));
            assert_eq!(rust_growtable_capacity(h), 40);

            for i in 0..4u32 {
                let e = slot(h, i);
                assert_eq!(*e, TestElem { a: i, b: i * 10, c: i * 100 });
                // The identical pointer, not just identical contents -
                // this is the actual stable-address guarantee, not
                // merely "the data survived a copy."
                assert_eq!(e as *mut TestElem, first_chunk_ptrs[i as usize]);
            }
        }
    }

    #[test]
    fn refuses_to_grow_past_max_chunks() {
        let h = rust_growtable_create(12, 4, 2); // max 8 elements total
        unsafe {
            assert!(rust_growtable_ensure_capacity(h, 8));
            assert_eq!(rust_growtable_capacity(h), 8);
            assert!(!rust_growtable_ensure_capacity(h, 9));
            // Capacity from the chunks that DID succeed is kept, not
            // discarded, by a later failed grow attempt.
            assert_eq!(rust_growtable_capacity(h), 8);
        }
    }

    #[test]
    fn independent_tables_do_not_interfere() {
        let h1 = rust_growtable_create(12, 4, 4);
        let h2 = rust_growtable_create(8, 2, 4);
        assert_ne!(h1, h2);
        unsafe {
            assert!(rust_growtable_ensure_capacity(h1, 4));
            assert_eq!(rust_growtable_capacity(h2), 0);
            assert!(rust_growtable_ensure_capacity(h2, 2));
        }
        assert_eq!(rust_growtable_capacity(h1), 4);
        assert_eq!(rust_growtable_capacity(h2), 2);
    }

    #[test]
    fn invalid_handle_is_handled_not_undefined() {
        assert_eq!(rust_growtable_capacity(-1), 0);
        assert_eq!(rust_growtable_capacity(999), 0);
        assert!(rust_growtable_slot_ptr(-1, 0).is_null());
        unsafe {
            assert!(!rust_growtable_ensure_capacity(-1, 1));
        }
    }
}
