//! kernel/rust/pipe.rs - Phase 36: kernel pipes, NovaOS's first real
//! inter-process communication mechanism.
//!
//! Gap this fills: before this phase, processes had no way to
//! communicate with each other at all - no pipes, no shared memory,
//! no message queues, nothing. The existing "handle" abstraction
//! (kernel/arch/x86/cpu/syscall.c's open_files[] table) only ever
//! backed onto named files on disk. This is the first generic byte-
//! stream primitive that isn't VFS-backed - a real gap, not a rewrite
//! of anything that already worked, matching this project's standing
//! rule (kernel/rust/lib.rs's own header comment) that new kernel
//! work is attempted in Rust first.
//!
//! Why this particular subsystem suits Rust well, specifically: a
//! ring buffer is exactly the kind of code where a single off-by-one
//! in the index arithmetic is a classic kernel memory-safety bug
//! class (reading/writing one byte past the buffer, or corrupting an
//! unrelated kernel structure sitting right after it in memory) - a
//! class of bug this project's own C code works hard to guard against
//! elsewhere (e.g. heap.c's magic-number corruption detection, added
//! specifically because a stray overwrite is otherwise silent and
//! hard to diagnose). Every array access here goes through Rust's
//! ordinary indexing, which panics loudly on an out-of-bounds access
//! instead of silently reading/writing adjacent memory - the compiler
//! enforces the same property heap.c's magic number can only detect
//! after the fact.
//!
//! Design, and what's deliberately NOT in scope for this phase:
//!
//! - Fixed-capacity, statically-allocated pipe slots (MAX_PIPES of
//!   them, each PIPE_CAPACITY bytes) - no heap allocation, matching
//!   this kernel's existing preference for static/fixed-size kernel
//!   structures (MAX_OPEN_FILES, MAX_PROCESSES, MAX_CAPABILITIES all
//!   follow the same pattern) and avoiding the need to expose this
//!   project's C kmalloc()/kfree() across the Rust/C FFI boundary at
//!   all for this phase.
//!
//! - Genuinely non-blocking: pipe_read() returns immediately with a
//!   distinct "would block" result if the buffer is empty and the
//!   write end is still open, rather than blocking inside the syscall
//!   handler. This matches this kernel's own established pattern for
//!   every other syscall that could otherwise wait on something -
//!   SYS_READ_KEY and SYS_PING_START/POLL both make exactly this same
//!   choice, and for the same underlying reason documented in
//!   syscall.h: a syscall handler runs with interrupts disabled for
//!   its whole duration, so a genuinely blocking wait here would risk
//!   the entire kernel hanging, not just the calling process. A
//!   caller wanting to actually wait is expected to loop with
//!   SYS_YIELD between attempts, the same convention process_wait()
//!   and every other blocking wait in this kernel already uses one
//!   level up.
//!
//! - No blocking on a full write either, for the same reason - a
//!   short write (fewer bytes accepted than asked for) is returned
//!   instead, matching real Unix pipe semantics for a full pipe in
//!   non-blocking mode, and leaving "retry the remainder" to the
//!   caller.
//!
//! - No cross-process fd inheritance: a pipe's two handles are only
//!   ever visible to the process that created them (open_files[]
//!   entries are already owner_pid-gated, unchanged by this phase).
//!   This kernel's fork() (Phase 27) does not currently duplicate a
//!   parent's open_files[] entries into the child at all - a real,
//!   honest, pre-existing gap this phase doesn't attempt to close (it
//!   would need process_fork() itself to walk and duplicate
//!   open_files[] by owner_pid, a change to process-lifecycle code,
//!   not to pipes specifically). The natural, fully-general "shell
//!   pipeline between two independent processes" (`a | b`) therefore
//!   isn't reachable yet either - this phase proves the primitive
//!   itself works correctly, end to end through the real syscall
//!   path, not the shell integration on top of it.
//!
//! Concurrency note (updated in Phase 40): PIPES was originally
//! protected only by the documented assumption that syscalls run
//! with interrupts disabled for their entire duration, so nothing
//! else could run concurrently with syscall-handler code on this
//! single-core target - true, but not a *type-enforced* guarantee,
//! and silent about interrupt-context reentrancy specifically (two
//! different IRQ handlers touching the same state) and about any
//! future second CPU. Now wrapped in kernel/rust/spinlock.rs's
//! SpinLock<T> instead - the same access pattern as before
//! (`PIPES.lock()` in place of the old `unsafe { pipes() }` accessor)
//! but now a real, enforced lock rather than a comment asking every
//! future caller to already know and respect an assumption. No
//! observable behavior change today (nothing yet calls into this
//! module from interrupt context), but see spinlock.rs's own header
//! comment for why that's exactly the point - this makes the next
//! piece of code that *does* need to touch pipe state from an IRQ
//! handler correct by construction, not another thing to remember.

const MAX_PIPES: usize = 8;
const PIPE_CAPACITY: usize = 1024;

struct Pipe {
    buf: [u8; PIPE_CAPACITY],
    read_pos: usize,
    write_pos: usize,
    count: usize,
    in_use: bool,
    reader_open: bool,
    writer_open: bool,
}

impl Pipe {
    const fn new() -> Self {
        Pipe {
            buf: [0u8; PIPE_CAPACITY],
            read_pos: 0,
            write_pos: 0,
            count: 0,
            in_use: false,
            reader_open: false,
            writer_open: false,
        }
    }
}

const EMPTY_PIPE: Pipe = Pipe::new();
static PIPES: crate::spinlock::SpinLock<[Pipe; MAX_PIPES]> =
    crate::spinlock::SpinLock::new([EMPTY_PIPE; MAX_PIPES]);

/// Allocates a new pipe. Returns its id (0..MAX_PIPES), or -1 if every
/// pipe slot is already in use - this kernel's usual fixed-capacity-
/// exhaustion failure mode (matching SYS_OPEN's own "open file table
/// full" case), not a real, expected condition in ordinary use.
#[no_mangle]
pub extern "C" fn rust_pipe_create() -> i32 {
    let mut pipes = PIPES.lock();
    for (i, p) in pipes.iter_mut().enumerate() {
        if !p.in_use {
            p.in_use = true;
            p.read_pos = 0;
            p.write_pos = 0;
            p.count = 0;
            p.reader_open = true;
            p.writer_open = true;
            return i as i32;
        }
    }
    -1
}

/// Reads up to `max_len` bytes from pipe `id` into `buf`.
///
/// Returns:
///  - `> 0`: the number of bytes actually read (1..=max_len)
///  - `0`: real end-of-stream - the buffer was empty AND the write
///    end has already been closed. There will never be more data.
///  - `-1`: `id` doesn't refer to a currently-open pipe.
///  - `-2`: would block - the buffer is empty but the write end is
///    still open, so more data may still arrive. Distinct from `0`
///    deliberately: a caller must be able to tell "nothing more is
///    ever coming" apart from "nothing is here *yet*", the same
///    distinction real POSIX read()/EOF vs EAGAIN makes.
///
/// # Safety
/// `buf` must be valid for writes of `max_len` bytes. Callers in this
/// codebase pass a syscall argument register's value straight through
/// (see syscall.c's handle_read()) without validating it themselves
/// either - consistent with, not a new gap beyond, this kernel's
/// existing syscall argument handling throughout (e.g. handle_open()
/// dereferences regs->ebx as a filename pointer the same way).
#[no_mangle]
pub unsafe extern "C" fn rust_pipe_read(id: i32, buf: *mut u8, max_len: u32) -> i32 {
    if id < 0 || (id as usize) >= MAX_PIPES || buf.is_null() {
        return -1;
    }
    let mut pipes = PIPES.lock();
    let p = &mut pipes[id as usize];
    if !p.in_use {
        return -1;
    }
    if p.count == 0 {
        return if p.writer_open { -2 } else { 0 };
    }

    let to_copy = core::cmp::min(max_len as usize, p.count);
    let out = core::slice::from_raw_parts_mut(buf, to_copy);
    for slot in out.iter_mut() {
        *slot = p.buf[p.read_pos];
        p.read_pos = (p.read_pos + 1) % PIPE_CAPACITY;
    }
    p.count -= to_copy;
    to_copy as i32
}

/// Writes up to `len` bytes from `buf` into pipe `id`.
///
/// Returns:
///  - `>= 0`: the number of bytes actually accepted (0..=len). May be
///    less than `len` if the pipe's buffer filled up first - a short
///    write, matching real Unix non-blocking pipe-write semantics,
///    rather than blocking for space or discarding data silently.
///  - `-1`: `id` doesn't refer to a currently-open pipe, or its read
///    end has already been closed (a genuine "broken pipe" - nothing
///    will ever read what's written from here on, so refusing outright
///    is more honest than silently accepting and discarding bytes).
///
/// # Safety
/// `buf` must be valid for reads of `len` bytes - see rust_pipe_read's
/// safety note; the same reasoning applies here.
#[no_mangle]
pub unsafe extern "C" fn rust_pipe_write(id: i32, buf: *const u8, len: u32) -> i32 {
    if id < 0 || (id as usize) >= MAX_PIPES || buf.is_null() {
        return -1;
    }
    let mut pipes = PIPES.lock();
    let p = &mut pipes[id as usize];
    if !p.in_use || !p.reader_open {
        return -1;
    }

    let space = PIPE_CAPACITY - p.count;
    let to_copy = core::cmp::min(len as usize, space);
    let input = core::slice::from_raw_parts(buf, to_copy);
    for &byte in input.iter() {
        p.buf[p.write_pos] = byte;
        p.write_pos = (p.write_pos + 1) % PIPE_CAPACITY;
    }
    p.count += to_copy;
    to_copy as i32
}

/// Closes one end of pipe `id` - `is_read_end != 0` for the read end,
/// `0` for the write end. Once BOTH ends have been closed, the pipe
/// slot itself is freed for reuse by a future rust_pipe_create() call.
/// Closing an already-closed end, or an invalid/nonexistent id, is a
/// harmless no-op - matching handle_close()'s own tolerance of being
/// called on an already-closed or invalid handle.
#[no_mangle]
pub extern "C" fn rust_pipe_close(id: i32, is_read_end: i32) {
    if id < 0 || (id as usize) >= MAX_PIPES {
        return;
    }
    let mut pipes = PIPES.lock();
    let p = &mut pipes[id as usize];
    if !p.in_use {
        return;
    }
    if is_read_end != 0 {
        p.reader_open = false;
    } else {
        p.writer_open = false;
    }
    if !p.reader_open && !p.writer_open {
        p.in_use = false;
    }
}
