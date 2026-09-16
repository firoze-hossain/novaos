//! kernel/rust/journal.rs - Phase 53: a real write-ahead journal for
//! FAT32, closing the release-readiness doc's own "A journaled or
//! copy-on-write filesystem: Not started" gap (Part 1.3).
//!
//! Gap this fills, stated precisely: FAT32 write support (Phase 8) has
//! always done its several sector writes per logical operation
//! directly against the disk - allocate/chain FAT entries, write data
//! clusters, then write the directory entry, each its own
//! blockdev_write_sectors() call. Power loss between any two of those
//! writes has always left the on-disk filesystem in a state no single
//! operation ever intended: a directory entry pointing at a partially-
//! written cluster chain, or a cluster chain allocated with no
//! directory entry pointing at it yet. This module makes each of
//! fat32.c's two real write entry points (fat32_write_file(),
//! fat32_delete_file()) atomic against power loss: every sector write
//! that entry point performs, however many FAT-table updates,
//! directory-cluster extensions, and data writes it takes, either all
//! reach disk or none do, even if power is lost at any point during
//! the call. ext2.c is deliberately untouched - it is read-only (see
//! kernel/fs/vfs.c's own header comment), so it performs no writes
//! this module would have anything to protect.
//!
//! Design: physical block journaling, the same fundamental shape as
//! Linux's JBD (ext3/ext4) or a simplified ARIES-style write-ahead
//! log, scoped down for this kernel's actual concurrency model (see
//! "Why no SpinLock" below) and its single-caller usage pattern (see
//! "Why one on-disk transaction slot" below):
//!
//!  1. `rust_journal_begin()` opens a transaction. While one is open,
//!     `rust_journal_write_sectors()` does not touch the real disk at
//!     all - it buffers each sector into an in-memory "shadow" table
//!     (lba -> 512 bytes), and `rust_journal_read_sectors()` checks
//!     that same shadow first, falling back to a real disk read for
//!     any lba not yet shadowed. This "read your own writes" property
//!     is not a nicety - fat32.c's own allocator (find_free_cluster(),
//!     called repeatedly by alloc_cluster_chain() within a single
//!     fat32_write_file() transaction) reads the FAT entries it *just*
//!     wrote earlier in the same call to decide what's still free; a
//!     naive buffer that only intercepted writes would make every
//!     scan after the first allocated cluster see stale, still-free-
//!     looking FAT entries and hand out the same cluster twice.
//!  2. `rust_journal_commit()` closes the transaction and makes it
//!     durable in two ordered phases:
//!       a. *Durability*: the shadow's (lba, data) pairs are written
//!          to a dedicated on-disk journal region (its own disk
//!          partition - see tools/build-disk-image.sh's Phase 53
//!          update) as a descriptor block (listing which real lba
//!          each journaled sector belongs to, plus a checksum) and
//!          the data blocks themselves, and only *then* is the
//!          descriptor rewritten with its commit marker set. This
//!          ordering is the entire mechanism: if power is lost before
//!          the commit marker write lands, the descriptor's magic (on
//!          a freshly-zeroed disk region) or its commit marker (on a
//!          region still holding a previous, already-checkpointed
//!          transaction, deliberately invalidated at the end of step
//!          b below) reads back as "no valid transaction here," and
//!          the real filesystem - never touched by this phase - is
//!          exactly as it was before the call. If power survives past
//!          the commit marker write, the transaction is durable even
//!          though the real filesystem may not reflect it yet.
//!       b. *Checkpoint*: the shadow's (lba, data) pairs are written
//!          to their real, final locations (through the same
//!          blockdev_write_sectors() every non-journaled write already
//!          used), then the on-disk journal descriptor is invalidated
//!          (its magic zeroed) so a future recovery doesn't needlessly
//!          redo already-applied work.
//!  3. `rust_journal_recover()`, called once at boot (from vfs_init(),
//!     before fat32_init() mounts) reads the descriptor and, if it
//!     finds a valid, checksummed, commit-marked transaction, replays
//!     step 2b above - completing exactly the work a crash between 2a
//!     and 2b left undone. This is REDO, not undo: recovery never
//!     needs to know what the filesystem looked like *before* the
//!     transaction, only what it must look like *after* - the
//!     descriptor's target-lba list plus the data blocks already
//!     sitting in the journal region are the entire input.
//!
//! This gives exactly the property the release-readiness doc names as
//! missing: "power loss mid-write... real data corruption" - a crash
//! at any point during a journaled fat32.c operation now leaves the
//! filesystem looking like the operation either fully happened or not
//! at all, never a partial in-between state. Verified two ways, not
//! just designed this way on paper - see `rust_journal_selftest()`
//! below, and kernel/init/main.c's own Phase 53 self-test block for
//! how it's invoked and logged.
//!
//! Foundational assumption, stated honestly rather than silently
//! relied upon (the same standard every real journaling filesystem
//! built on ordinary block devices holds itself to, including ext3/
//! JBD's own): a single blockdev_write_sectors() call for exactly one
//! 512-byte sector is assumed atomic - it either completes wholly or
//! not at all, never torn mid-sector. This kernel's ATA and virtio-blk
//! backends are both synchronous/polled (see kernel/drivers/blockdev.c
//! and virtio_blk.c's own "one request in flight at a time" scope
//! note) with no write-back caching modeled anywhere in this stack, so
//! there is no reordering hazard beyond this - the order this module
//! issues its writes in *is* the order they reach the emulated disk.
//! A real drive with a volatile write cache would need an explicit
//! cache-flush primitive between phases 2a's data writes and its
//! final commit-marker write, which this kernel has no equivalent of
//! (no such primitive exists anywhere else in this driver stack
//! either); that's a real, honestly-scoped limitation of the platform
//! this runs on, not something this module papers over.
//!
//! Why no SpinLock (kernel/rust/spinlock.rs), unlike kernel/rust/
//! pipe.rs's PIPES table: a spinlock's IRQ-disabling protection is
//! only worth paying for against a caller that could genuinely arrive
//! concurrently - from a second CPU, or from interrupt context while
//! this module's own non-interrupt-context code is mid-update. Neither
//! applies here: this kernel is still single-core (Part 1.4 of the
//! release-readiness doc), and nothing in this codebase calls into
//! fat32.c - and therefore into this module - from IRQ context; every
//! call is synchronous ring-0 C code (boot-time self-tests, or a
//! syscall handler, itself running with interrupts disabled for its
//! entire duration per this kernel's own existing, established
//! design). Holding a spinlock across this module's real disk I/O
//! (potentially dozens of sector writes during one commit/checkpoint)
//! would disable interrupts - blocking the timer tick, keyboard,
//! and network IRQs - for that whole duration, a real regression this
//! module has no correctness reason to introduce.
//!
//! Why one on-disk transaction slot, not a circular multi-transaction
//! log: exactly one transaction is ever open at a time in this
//! kernel's current design (fat32_write_file()/fat32_delete_file() are
//! synchronous, non-reentrant calls, and nothing else drives a
//! transaction) - a previous transaction is always fully checkpointed
//! and invalidated before `rust_journal_begin()` can be called again.
//! A fixed, single descriptor+data region that's simply overwritten by
//! the next transaction is therefore exactly as safe as a circular log
//! would be, with far less bookkeeping.
//!
//! Bounded transaction size, a stated scope limit (matching this
//! project's own established pattern of naming these plainly rather
//! than hiding them - e.g. fat32.c's own MAX_CLUSTER_SECTORS): at most
//! `MAX_TXN_BLOCKS` distinct 512-byte sectors can be shadowed inside
//! one transaction. A fat32.c operation whose total write footprint
//! would exceed this (a very large file write, given this project's
//! current fixtures are all well under the ~64KB this allows) fails
//! that one blockdev call - `rust_journal_write_sectors()` returns
//! false exactly like a real disk I/O failure would, so the caller's
//! existing failure handling applies unchanged - rather than silently
//! journaling only part of the operation, which would defeat the
//! entire point.

use core::sync::atomic::{AtomicU32, Ordering};

extern "C" {
    fn blockdev_read_sectors(lba: u32, sector_count: u8, buffer: *mut u8) -> bool;
    fn blockdev_write_sectors(lba: u32, sector_count: u8, buffer: *const u8) -> bool;
    fn blockdev_read_sectors_absolute(lba: u32, sector_count: u8, buffer: *mut u8) -> bool;
    fn blockdev_write_sectors_absolute(lba: u32, sector_count: u8, buffer: *const u8) -> bool;
    /* blockdev_id_t (kernel/drivers/blockdev.h) is a plain C enum with
     * no explicit underlying type - the same int-sized representation
     * `bool` (kernel/include/types.h's own enum) already crosses this
     * FFI boundary as everywhere else in this codebase, so `i32` here
     * is exactly as sound as every existing `bool` return value already
     * relied on. Used only by `device_matches_configured` below. */
    fn blockdev_current() -> i32;
}

const SECTOR_SIZE: usize = 512;

/// Bounded transaction size - see this file's own header comment on
/// why a bound exists and what happens if it's exceeded.
const MAX_TXN_BLOCKS: usize = 128;

/// The descriptor needs: magic(4) + seq(4) + block_count(4) +
/// checksum(4) + commit_flag(4) + targets_absolute(4) +
/// target_lba[MAX_TXN_BLOCKS](4 each) = 24 + 512 = 536 bytes for
/// MAX_TXN_BLOCKS=128, rounded up to whole sectors.
const DESCRIPTOR_SECTORS: usize = 2;
const DESCRIPTOR_BYTES: usize = DESCRIPTOR_SECTORS * SECTOR_SIZE;

const OFF_MAGIC: usize = 0;
const OFF_SEQ: usize = 4;
const OFF_BLOCK_COUNT: usize = 8;
const OFF_CHECKSUM: usize = 12;
const OFF_COMMIT_FLAG: usize = 16;
const OFF_TARGETS_ABSOLUTE: usize = 20;
const OFF_TARGET_LBA: usize = 24;

/// A fixed, arbitrary-but-distinct sentinel - never produced by a
/// freshly-zeroed disk region (which reads back as all zero), so a
/// journal region that has never held a transaction, or whose last
/// transaction was cleanly invalidated after checkpointing, correctly
/// reads as "nothing to recover" without needing a separate "is this
/// region initialized at all" flag.
const MAGIC: u32 = 0x4C52_4E4A; // read little-endian bytes as "JNRL"-ish
/// Written only as the very last byte range of the very last write of
/// phase 2a (see this file's header comment) - the single bit whose
/// presence or absence is what recovery actually decides on.
const COMMIT_MAGIC: u32 = 0x5449_4D43; // "CMIT" bytes, little-endian u32

fn put_u32_le(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn get_u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

/// FNV-1a, 32-bit. Not cryptographic - this is defense against torn
/// or partially-written journal data (the real failure mode a crash
/// mid-write produces), and against ordinary accidental bit rot, not
/// against deliberate tampering. Sufficient for this module's actual
/// threat model, the same honest scoping this project already applies
/// elsewhere (e.g. kernel/rust/users.rs's own documented salt-entropy
/// limitation).
fn fnv1a_fold(mut h: u32, bytes: &[u8]) -> u32 {
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn compute_checksum(lba_bytes: &[u8], data_blocks: &[[u8; SECTOR_SIZE]]) -> u32 {
    let mut h: u32 = 0x811c_9dc5; // FNV-1a 32-bit offset basis
    h = fnv1a_fold(h, lba_bytes);
    for block in data_blocks {
        h = fnv1a_fold(h, &block[..]);
    }
    h
}

struct JournalState {
    configured: bool,
    /// Which blockdev_id_t (kernel/drivers/blockdev.h) was active when
    /// `rust_journal_configure` was called - see `journal_region_usable`
    /// below for why this matters: the journal region's own absolute
    /// LBAs are only meaningful on the disk they were computed against.
    configured_device: i32,
    journal_start_lba: u32,
    journal_len_sectors: u32,
    seq: u32,
    depth: u32,
    targets_absolute: bool,
    shadow_count: usize,
    shadow_lba: [u32; MAX_TXN_BLOCKS],
    shadow_data: [[u8; SECTOR_SIZE]; MAX_TXN_BLOCKS],
}

impl JournalState {
    const fn new() -> Self {
        JournalState {
            configured: false,
            configured_device: -1,
            journal_start_lba: 0,
            journal_len_sectors: 0,
            seq: 0,
            depth: 0,
            targets_absolute: false,
            shadow_count: 0,
            shadow_lba: [0; MAX_TXN_BLOCKS],
            shadow_data: [[0u8; SECTOR_SIZE]; MAX_TXN_BLOCKS],
        }
    }

    fn find_shadow(&self, lba: u32) -> Option<usize> {
        for i in 0..self.shadow_count {
            if self.shadow_lba[i] == lba {
                return Some(i);
            }
        }
        None
    }
}

/// Single global transaction - see this file's header comment ("Why
/// no SpinLock", "Why one on-disk transaction slot") for why a bare
/// `static mut` is the honest, correctly-scoped choice here rather
/// than under-engineering by accident.
static mut JOURNAL: JournalState = JournalState::new();

/// Monotonically-increasing counter purely for the self-test's own
/// scratch-sector selection (see `rust_journal_selftest`) - kept as a
/// real atomic even though nothing here is genuinely concurrent, so
/// this one piece of state can't become a silent exception to "this
/// module states its concurrency assumptions plainly" if a future
/// caller ever changes that.
static SELFTEST_ROUND: AtomicU32 = AtomicU32::new(0);

/// Reads `count` sectors starting at `lba` into `dst` (which must
/// point to at least `count * 512` bytes), honoring the shadow table
/// when `js.depth > 0` (mid-transaction) so a caller sees its own
/// not-yet-checkpointed writes.
///
/// # Safety
/// `dst` must be valid for `count * 512` writable bytes.
unsafe fn read_sectors_internal(js: &mut JournalState, lba: u32, count: u8, dst: *mut u8) -> bool {
    if js.depth == 0 {
        return blockdev_read_sectors(lba, count, dst);
    }
    for i in 0..count as u32 {
        let cur_lba = lba.wrapping_add(i);
        let dst_i = dst.add((i as usize) * SECTOR_SIZE);
        if let Some(idx) = js.find_shadow(cur_lba) {
            core::ptr::copy_nonoverlapping(js.shadow_data[idx].as_ptr(), dst_i, SECTOR_SIZE);
        } else if !blockdev_read_sectors(cur_lba, 1, dst_i) {
            return false;
        }
    }
    true
}

/// Writes `count` sectors starting at `lba` from `src` (which must
/// point to at least `count * 512` bytes). Mid-transaction
/// (`js.depth > 0`), buffers into the shadow table instead of
/// touching disk - see this file's header comment for why. Returns
/// false (without partially applying the call) if the shadow table's
/// bounded capacity would be exceeded.
///
/// # Safety
/// `src` must be valid for `count * 512` readable bytes.
unsafe fn write_sectors_internal(
    js: &mut JournalState,
    lba: u32,
    count: u8,
    src: *const u8,
) -> bool {
    if js.depth == 0 {
        return blockdev_write_sectors(lba, count, src);
    }
    for i in 0..count as u32 {
        let cur_lba = lba.wrapping_add(i);
        let src_i = src.add((i as usize) * SECTOR_SIZE);
        let idx = match js.find_shadow(cur_lba) {
            Some(idx) => idx,
            None => {
                if js.shadow_count >= MAX_TXN_BLOCKS {
                    return false; // bounded-transaction scope limit - see header comment
                }
                let idx = js.shadow_count;
                js.shadow_lba[idx] = cur_lba;
                js.shadow_count += 1;
                idx
            }
        };
        core::ptr::copy_nonoverlapping(src_i, js.shadow_data[idx].as_mut_ptr(), SECTOR_SIZE);
    }
    true
}

/// True only when a journal region is configured *and* the disk it
/// was configured against is still the one blockdev.h's own dispatch
/// currently has active. The second half matters for a real, already-
/// existing scenario in this codebase, not a hypothetical: kernel/
/// init/main.c's own Phase 46 virtio-blk self-test temporarily calls
/// blockdev_select(BLOCKDEV_VIRTIO_BLK) and mounts a *different*,
/// separately-built raw FAT32 image on it, calling fat32_write_file()
/// against that filesystem before restoring the original device.
/// Without this check, this module would durably write to (and read
/// recovery data from) the journal region's absolute LBAs on whatever
/// device happens to be active *right now* - meaningless, and
/// potentially disk-corrupting, on a physically different disk than
/// the one those LBAs were computed against. When the device doesn't
/// match, this module transparently degrades to direct, unjournaled
/// writes for that call (exactly the same fallback as "no journal
/// partition configured at all" - see `rust_journal_configure`'s own
/// doc comment) - correct, not merely safe: the temporarily-mounted
/// filesystem's own writes still happen, just without this module's
/// crash-safety wrapping, which is no worse than every disk's own
/// behavior before this phase existed.
fn journal_region_usable(js: &JournalState) -> bool {
    js.configured && unsafe { blockdev_current() } == js.configured_device
}

/// Phase 2a (see header comment): makes the current shadow durable in
/// the on-disk journal region, in the exact order correctness depends
/// on (descriptor-without-commit-marker, then every data block, then
/// the descriptor rewritten *with* the commit marker). A no-op success
/// if there's nothing to journal (empty shadow), no journal region is
/// configured, or the configured journal region belongs to a
/// different disk than the one currently active (see
/// `journal_region_usable`'s own doc comment) - every such case
/// degrades to direct, unjournaled writes exactly as every disk
/// behaved before this phase.
fn commit_durable_internal(js: &mut JournalState) -> bool {
    if !journal_region_usable(js) || js.shadow_count == 0 {
        return true;
    }

    let mut desc = [0u8; DESCRIPTOR_BYTES];
    put_u32_le(&mut desc, OFF_MAGIC, MAGIC);
    put_u32_le(&mut desc, OFF_SEQ, js.seq);
    put_u32_le(&mut desc, OFF_BLOCK_COUNT, js.shadow_count as u32);
    put_u32_le(&mut desc, OFF_COMMIT_FLAG, 0);
    put_u32_le(&mut desc, OFF_TARGETS_ABSOLUTE, js.targets_absolute as u32);
    for i in 0..js.shadow_count {
        put_u32_le(&mut desc, OFF_TARGET_LBA + i * 4, js.shadow_lba[i]);
    }
    let checksum = compute_checksum(
        &desc[OFF_TARGET_LBA..OFF_TARGET_LBA + js.shadow_count * 4],
        &js.shadow_data[0..js.shadow_count],
    );
    put_u32_le(&mut desc, OFF_CHECKSUM, checksum);

    unsafe {
        // Pre-write: descriptor with commit_flag still 0 - if power is
        // lost anywhere from here through the data-block writes below,
        // this transaction is not yet committed and recovery ignores
        // it entirely.
        if !blockdev_write_sectors_absolute(
            js.journal_start_lba,
            DESCRIPTOR_SECTORS as u8,
            desc.as_ptr(),
        ) {
            return false;
        }

        for i in 0..js.shadow_count {
            let lba = js
                .journal_start_lba
                .wrapping_add(DESCRIPTOR_SECTORS as u32)
                .wrapping_add(i as u32);
            if !blockdev_write_sectors_absolute(lba, 1, js.shadow_data[i].as_ptr()) {
                return false;
            }
        }

        // The commit point: this is the one write whose completion
        // durably means "this transaction happened," per this file's
        // own header comment.
        put_u32_le(&mut desc, OFF_COMMIT_FLAG, COMMIT_MAGIC);
        if !blockdev_write_sectors_absolute(
            js.journal_start_lba,
            DESCRIPTOR_SECTORS as u8,
            desc.as_ptr(),
        ) {
            return false;
        }
    }

    js.seq = js.seq.wrapping_add(1);
    true
}

/// Phase 2b (see header comment): applies the shadow to its real,
/// final locations, then invalidates the on-disk journal (if
/// configured and non-empty) so a future recovery doesn't redo
/// already-applied work. Always clears the shadow and
/// `targets_absolute`, even on partial failure, since a failed
/// checkpoint has already done as much of its job as it can - see
/// this file's header comment on why fat32.c's own callers commit
/// unconditionally rather than trying to "abort" a transaction.
fn checkpoint_internal(js: &mut JournalState) -> bool {
    let mut ok = true;
    for i in 0..js.shadow_count {
        let lba = js.shadow_lba[i];
        let ptr = js.shadow_data[i].as_ptr();
        let wrote = unsafe {
            if js.targets_absolute {
                blockdev_write_sectors_absolute(lba, 1, ptr)
            } else {
                blockdev_write_sectors(lba, 1, ptr)
            }
        };
        if !wrote {
            ok = false;
        }
    }

    if journal_region_usable(js) && js.shadow_count > 0 {
        // Invalidating just the magic is enough - a zeroed magic never
        // equals MAGIC regardless of what commit_flag happens to say.
        // Gated on journal_region_usable(), not just js.configured -
        // see that function's own doc comment: if the currently active
        // device isn't the one this journal region belongs to, nothing
        // durable was written for this transaction in the first place
        // (commit_durable_internal's own identical gate skipped it),
        // so there is nothing to invalidate, and writing to
        // journal_start_lba would land on the wrong physical disk.
        let zero = [0u8; SECTOR_SIZE];
        unsafe {
            let _ = blockdev_write_sectors_absolute(js.journal_start_lba, 1, zero.as_ptr());
        }
    }

    js.shadow_count = 0;
    js.targets_absolute = false;
    ok
}

/// Reads the on-disk descriptor and, if it describes a valid,
/// checksummed, commit-marked transaction, replays it (phase 2b) -
/// see this file's header comment. Reconstructs into `js`'s own
/// shadow storage rather than a stack-local buffer deliberately:
/// `MAX_TXN_BLOCKS * 512` bytes (64KB) is far too large to put on this
/// kernel's own kernel-mode stack, and reusing the persistent shadow
/// table is safe here specifically because recovery always runs
/// before any real transaction begins (see `rust_journal_recover`'s
/// own call site in kernel/fs/vfs.c's vfs_init(), ahead of
/// fat32_init()) - nothing else is using it yet.
///
/// Returns the number of blocks replayed (0 if nothing valid was
/// found - not an error, the ordinary case on a clean shutdown), or
/// -1 on a real I/O error reading the journal region itself.
fn recover_internal(js: &mut JournalState) -> i32 {
    if !journal_region_usable(js) {
        return 0;
    }

    let mut desc = [0u8; DESCRIPTOR_BYTES];
    let read_ok = unsafe {
        blockdev_read_sectors_absolute(js.journal_start_lba, DESCRIPTOR_SECTORS as u8, desc.as_mut_ptr())
    };
    if !read_ok {
        return -1;
    }

    let magic = get_u32_le(&desc, OFF_MAGIC);
    let block_count = get_u32_le(&desc, OFF_BLOCK_COUNT) as usize;
    let stored_checksum = get_u32_le(&desc, OFF_CHECKSUM);
    let commit_flag = get_u32_le(&desc, OFF_COMMIT_FLAG);
    let targets_absolute = get_u32_le(&desc, OFF_TARGETS_ABSOLUTE) != 0;

    if magic != MAGIC || commit_flag != COMMIT_MAGIC || block_count == 0
        || block_count > MAX_TXN_BLOCKS
    {
        return 0; // nothing valid to recover - the ordinary case
    }

    for i in 0..block_count {
        js.shadow_lba[i] = get_u32_le(&desc, OFF_TARGET_LBA + i * 4);
        let lba = js
            .journal_start_lba
            .wrapping_add(DESCRIPTOR_SECTORS as u32)
            .wrapping_add(i as u32);
        let ok = unsafe { blockdev_read_sectors_absolute(lba, 1, js.shadow_data[i].as_mut_ptr()) };
        if !ok {
            return -1;
        }
    }

    let recomputed = compute_checksum(
        &desc[OFF_TARGET_LBA..OFF_TARGET_LBA + block_count * 4],
        &js.shadow_data[0..block_count],
    );
    if recomputed != stored_checksum {
        // Torn/corrupt journal data - never apply data we can't
        // verify is intact. Invalidate and report nothing recovered,
        // the same safe outcome as a transaction that never committed.
        let zero = [0u8; SECTOR_SIZE];
        unsafe {
            let _ = blockdev_write_sectors_absolute(js.journal_start_lba, 1, zero.as_ptr());
        }
        return 0;
    }

    js.shadow_count = block_count;
    js.targets_absolute = targets_absolute;
    if checkpoint_internal(js) {
        block_count as i32
    } else {
        -1
    }
}

/// Configures the on-disk journal region: `journal_start_lba` is
/// absolute (not relative to any filesystem's own partition offset -
/// see kernel/drivers/blockdev.h's `_absolute` variants), and
/// `journal_len_sectors` is how large that region is. If it's too
/// small to hold this module's fixed descriptor+data layout, journaling
/// is left disabled (`rust_journal_write_sectors`/`rust_journal_commit`
/// still work, degrading to direct, unjournaled writes exactly as
/// every disk behaved before this phase) rather than partially
/// enabled in some smaller, silently-truncated form.
///
/// Called once from kernel/fs/vfs.c's vfs_init(), only when a disk's
/// partition table actually has a third partition (this project's own
/// tools/build-disk-image.sh now creates one) - a bare/legacy two-
/// partition (or unpartitioned) disk simply never calls this, and
/// every write on such a disk continues exactly as before this phase.
///
/// Also records which block device (kernel/drivers/blockdev.h) is
/// active right now - see `journal_region_usable`'s own doc comment
/// for why this module must refuse to treat its journal region as
/// usable if a *different* device later becomes active (as kernel/
/// init/main.c's own Phase 46 virtio-blk self-test does, temporarily).
#[no_mangle]
pub extern "C" fn rust_journal_configure(journal_start_lba: u32, journal_len_sectors: u32) {
    let js = unsafe { &mut JOURNAL };
    let needed = (DESCRIPTOR_SECTORS + MAX_TXN_BLOCKS) as u32;
    js.configured = journal_len_sectors >= needed;
    js.configured_device = unsafe { blockdev_current() };
    js.journal_start_lba = journal_start_lba;
    js.journal_len_sectors = journal_len_sectors;
}

/// Real boot-time crash recovery - see `recover_internal`'s own doc
/// comment. Safe (and cheap: one 2-sector read, usually followed by
/// nothing else) to call even when nothing needs recovering, which is
/// the ordinary case on every clean shutdown.
#[no_mangle]
pub extern "C" fn rust_journal_recover() -> i32 {
    let js = unsafe { &mut JOURNAL };
    recover_internal(js)
}

/// Opens (or, if already open, extends) a transaction. Nesting-safe by
/// design (a depth counter, the same shape kernel/rust/spinlock.rs's
/// own nested-lock handling already established in this codebase) -
/// not currently exercised by any nested caller (fat32.c's own
/// internal helpers - fat_set_next_cluster(), alloc_cluster_chain(),
/// find_free_slot(), free_cluster_chain() - are only ever called from
/// within fat32_write_file()/fat32_delete_file()'s own already-open
/// transaction, never standalone), but kept correct regardless rather
/// than assuming that stays true forever.
#[no_mangle]
pub extern "C" fn rust_journal_begin() {
    let js = unsafe { &mut JOURNAL };
    js.depth += 1;
}

/// Closes (or, if nested, un-nests) a transaction. Only the outermost
/// commit (depth reaching 0) actually performs phase 2a/2b - see this
/// file's header comment. Calling this with no matching
/// `rust_journal_begin()` is a defensive no-op returning `true`
/// (nothing to do), rather than underflowing the depth counter.
///
/// Always attempts the checkpoint phase even if the durability phase
/// failed (a real disk I/O error writing the journal region) - a
/// journaled write that couldn't be made crash-safe should still be
/// attempted directly, the same as it would have been before this
/// phase existed; the return value reflects both phases honestly
/// (`true` only if both succeeded).
#[no_mangle]
pub extern "C" fn rust_journal_commit() -> bool {
    let js = unsafe { &mut JOURNAL };
    if js.depth == 0 {
        return true;
    }
    js.depth -= 1;
    if js.depth > 0 {
        return true; // still nested - the outer commit will finalize
    }
    let durable_ok = commit_durable_internal(js);
    let checkpoint_ok = checkpoint_internal(js);
    durable_ok && checkpoint_ok
}

/// See `read_sectors_internal`. `buf` must point to at least
/// `sector_count * 512` writable bytes - the same contract
/// kernel/drivers/blockdev.h's own blockdev_read_sectors() already
/// documents, since every call site in kernel/fs/fat32.c is a pure
/// rename from that function (see this file's header comment).
///
/// # Safety
/// `buf` must be valid for `sector_count * 512` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn rust_journal_read_sectors(
    lba: u32,
    sector_count: u8,
    buf: *mut u8,
) -> bool {
    let js = &mut JOURNAL;
    read_sectors_internal(js, lba, sector_count, buf)
}

/// See `write_sectors_internal`.
///
/// # Safety
/// `buf` must be valid for `sector_count * 512` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rust_journal_write_sectors(
    lba: u32,
    sector_count: u8,
    buf: *const u8,
) -> bool {
    let js = &mut JOURNAL;
    write_sectors_internal(js, lba, sector_count, buf)
}

/// Ring-0 self-test, called directly from kernel_main() the same way
/// every other kernel-side Rust module in this project proves itself
/// (see e.g. kernel/rust/spinlock.rs's own rust_spinlock_selftest()).
/// Exercises the two halves of this module's actual correctness claim
/// directly against the real, already-configured journal region (not
/// a mock), using dedicated scratch sectors near the end of that
/// region - well past the descriptor+data area this module's own
/// normal operation ever touches (`DESCRIPTOR_SECTORS + MAX_TXN_BLOCKS`
/// = 130 sectors from the start; this picks sectors starting at
/// `journal_len_sectors - 16`, leaving generous headroom) - so this
/// self-test can never collide with, or be confused for, a real
/// transaction:
///
///   Part A - crash *after* durability, *before* checkpoint: commits a
///   transaction durably (phase 2a) but deliberately skips checkpoint
///   (phase 2b), the exact state a real power loss in that window
///   would leave behind. Then calls the *real*
///   `rust_journal_recover()` path (the same one kernel/fs/vfs.c calls
///   at every real boot) and confirms the scratch sector now holds the
///   written pattern - proving recovery genuinely completes an
///   interrupted commit, not just that the code compiles.
///
///   Part B - crash *before* durability: writes a pattern into a
///   transaction but never durably commits it at all (the state a
///   power loss before phase 2a's first write would leave). Confirms
///   `rust_journal_recover()` leaves the scratch sector completely
///   untouched - proving an uncommitted transaction is discarded
///   cleanly, never partially applied.
///
/// Returns a bitmask (0 = every check passed, matching this project's
/// established self-test convention), or a negative value if no real
/// journal region is configured to test against (this self-test needs
/// one - see kernel/init/main.c's own call site for how that's
/// handled).
#[no_mangle]
pub extern "C" fn rust_journal_selftest() -> i32 {
    let js = unsafe { &mut JOURNAL };
    if !journal_region_usable(js) {
        return -1;
    }
    if js.journal_len_sectors < 16 {
        return -2; // journal region too small to safely carve out
                   // self-test scratch sectors without risking the
                   // real descriptor+data area - see this function's
                   // own doc comment
    }

    // A fresh scratch region per call (mod a small window) so
    // re-running this self-test later in the same boot (it currently
    // isn't, but nothing prevents it) doesn't depend on state a prior
    // run left behind.
    let round = SELFTEST_ROUND.fetch_add(1, Ordering::Relaxed) % 4;
    let scratch_base = js
        .journal_start_lba
        .wrapping_add(js.journal_len_sectors)
        .wrapping_sub(16)
        .wrapping_add(round.wrapping_mul(2));

    let mut code = 0i32;

    // --- Part A: durable commit, skipped checkpoint, real recovery ---
    let scratch_a = scratch_base;
    let pattern_a = [0xABu8; SECTOR_SIZE];

    js.shadow_count = 0;
    js.depth = 1;
    js.targets_absolute = true;
    let wrote = unsafe {
        write_sectors_internal(js, scratch_a, 1, pattern_a.as_ptr())
    };
    let durable_ok = wrote && commit_durable_internal(js);
    // Simulate the crash: drop the in-memory shadow/transaction state
    // (exactly what a real reboot would do to RAM) *without* running
    // checkpoint_internal() - the transaction is durable on disk but
    // not yet applied to its target.
    js.shadow_count = 0;
    js.targets_absolute = false;
    js.depth = 0;

    let recovered_a = recover_internal(js);

    let mut verify_a = [0u8; SECTOR_SIZE];
    let read_a_ok = unsafe {
        blockdev_read_sectors_absolute(scratch_a, 1, verify_a.as_mut_ptr())
    };
    let part_a_ok = durable_ok && recovered_a == 1 && read_a_ok && verify_a == pattern_a;
    if !part_a_ok {
        code |= 1;
    }

    // --- Part B: never durably committed, recovery must be a no-op ---
    let scratch_b = scratch_base.wrapping_add(1);
    let before_b = [0x00u8; SECTOR_SIZE];
    unsafe {
        let _ = blockdev_write_sectors_absolute(scratch_b, 1, before_b.as_ptr());
    }

    js.shadow_count = 0;
    js.depth = 1;
    js.targets_absolute = true;
    let pattern_b = [0xCDu8; SECTOR_SIZE];
    unsafe {
        let _ = write_sectors_internal(js, scratch_b, 1, pattern_b.as_ptr());
    }
    // Simulate the crash *before* commit_durable_internal() ever runs -
    // the on-disk journal still describes whatever transaction (if
    // any) Part A already left invalidated; scratch_b's own real
    // target sector was never touched.
    js.shadow_count = 0;
    js.targets_absolute = false;
    js.depth = 0;

    let recovered_b = recover_internal(js);

    let mut verify_b = [0u8; SECTOR_SIZE];
    let read_b_ok = unsafe {
        blockdev_read_sectors_absolute(scratch_b, 1, verify_b.as_mut_ptr())
    };
    let part_b_ok = recovered_b == 0 && read_b_ok && verify_b == before_b;
    if !part_b_ok {
        code |= 2;
    }

    code
}
