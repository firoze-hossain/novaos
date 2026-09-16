//! kernel/rust/crashdump.rs - Phase 54: a real, structured, persistent
//! crash dump, closing the release-readiness doc's own "A structured
//! crash-dump, not just a serial-port panic log: Not started" gap
//! (Part 1.3), the row right below Phase 53's journaling one.
//!
//! Gap this fills, stated precisely: every kernel_panic() call before
//! this phase did exactly one thing with the failure - format a short
//! human-readable line and send it to the serial port (kernel_log())
//! and the VGA console, then halt forever. That line is real, useful
//! information, but it is exactly as durable as the terminal window
//! or `-serial file:...` capture happened to be open at the time - a
//! panic nobody was watching for leaves nothing behind once the
//! machine is powered off. This module makes the CPU's own full
//! register state (when a real hardware exception caused the panic),
//! the panic reason, a best-effort call stack, and a timestamp durable
//! across a reboot: written to a dedicated disk region the instant a
//! panic happens (kernel/init/main.c's own kernel_panic_fault(), the
//! single real integration point - see that function for why), then
//! read back and reported once, automatically, the *next* time this
//! kernel boots - the actual, observable difference the release-
//! readiness doc's own wording draws ("I can tell you exactly what
//! broke" vs "the machine just stopped").
//!
//! Explicitly modeled on the same idea Windows' minidump format solves
//! (the doc's own named reference) - a small, structured, versioned
//! record capturing register/stack state at the moment of failure, not
//! a full memory image (a real minidump can optionally include much
//! more - loaded module lists, full heap contents; this kernel has no
//! loader/module list to capture and a full RAM dump has no realistic
//! home to write to in this project's current disk-image scope) -
//! scoped down to exactly the fields this kernel can actually capture
//! and a developer can actually use: what broke, where, and what the
//! call stack looked like at that instant.
//!
//! Design, in the order a real panic actually happens:
//!
//!  1. A CPU exception (page fault, GPF, divide error, ...) or an
//!     explicit, software-detected invariant violation (a heap
//!     corruption check, a size-agreement check, ...) calls into
//!     kernel/init/main.c's kernel_panic_fault() (or its thin wrapper,
//!     kernel_panic(), for the software-detected case - see that
//!     file's own comment for exactly which of this kernel's ~9 panic
//!     call sites has real CPU register state available and which
//!     doesn't).
//!  2. kernel_panic_fault() calls `rust_crashdump_write_panic()` before
//!     doing anything else (before even the existing serial/VGA
//!     logging) - if a *second* fault happens while writing the dump
//!     itself (a real risk this module takes seriously - see
//!     `walk_stack`'s own doc comment), the original panic's own
//!     serial-log line, at minimum, must not be lost by having already
//!     run after a dump write that never returns.
//!  3. This module encodes everything it was given - the reason
//!     string, the full CPU register snapshot (when one exists), the
//!     page-fault faulting address (CR2, when relevant), and a best-
//!     effort walk of the EBP frame-pointer chain - into one fixed-
//!     size, checksummed record, and writes it in a single sector
//!     write to a dedicated disk region (this kernel's fourth and
//!     - see "Why this is the last on-disk region this scheme can ever
//!     add" below - final MBR partition). A single 512-byte sector
//!     write is assumed atomic, the exact same foundational assumption
//!     kernel/rust/journal.rs's own header comment already states and
//!     justifies for this kernel's synchronous, non-write-caching ATA/
//!     virtio-blk backends - reused here, not re-argued.
//!  4. At the *next* boot, kernel/fs/vfs.c's vfs_init() (right next to
//!     where it already configures and recovers Phase 53's own
//!     journal) calls `rust_crashdump_configure()` then
//!     `rust_crashdump_check_and_report()`. If the record's magic and
//!     checksum are valid and it's still marked "pending" (not yet
//!     reported by an earlier boot), this kernel now has real, verified
//!     evidence of exactly what broke last time - decoded into a plain
//!     C struct (`crash_report_t`, kernel/fs/vfs.c's own mirror of this
//!     module's `CrashReport`) for kernel_log() to format and print,
//!     the same "Rust does the real logic, C does the thin display"
//!     split this project's other self-tests already use (e.g.
//!     kernel/rust/journal.rs's own bitmask results, formatted by
//!     kernel/init/main.c). The record is then rewritten with its
//!     "pending" field cleared (magic and content otherwise untouched)
//!     so a developer who's already seen and dealt with a crash isn't
//!     shown the exact same report on every subsequent boot forever -
//!     the same "don't repeat forever" UX a real crash reporter (or
//!     Windows' own post-bugcheck report) already provides.
//!
//! Why this is the last on-disk region this scheme can ever add,
//! stated directly rather than left implicit: kernel/fs/partition.h's
//! own `MAX_PARTITIONS` is 4 - a real, external constraint (plain MBR
//! supports at most four primary partition-table entries; a fifth
//! region would need MBR *extended* partitions, a different on-disk
//! structure this project's partition.c does not parse, or a switch to
//! GPT, which it also does not write). This phase's own disk region is
//! MBR partition 4 - FAT32 (1), ext2 (2), Phase 53's journal (3), and
//! now this (4) - so any future phase wanting its own dedicated,
//! partition-table-visible disk region will need one of those two
//! changes first. Not a blocker for this phase (one small crash-dump
//! record has no reason to want its own additional region anyway), but
//! worth stating plainly rather than discovering by surprise later.
//!
//! Why no SpinLock (kernel/rust/spinlock.rs), for a *different* reason
//! than kernel/rust/journal.rs's own (that module reasons from "never
//! called from IRQ context" - not true here, since a CPU exception
//! handler *is* IRQ-like context): `rust_crashdump_write_panic()` is
//! called at most once, ever, per boot, from a call chain that has
//! already disabled interrupts (kernel_panic_fault()'s own `cli`, or -
//! for the fault path - the interrupt gate itself, see kernel/arch/
//! x86/cpu/isr.c) and unconditionally halts forever immediately
//! afterward. There is no second caller to race against: by the time
//! this module's write path returns, the only thing that ever happens
//! next on this CPU is `hlt` in a loop. A SpinLock would protect state
//! this module never actually shares concurrently with anything.
//!
//! Foundational assumption, stated honestly (matching this project's
//! own established pattern - see kernel/rust/journal.rs's identical
//! section): a single-sector write is atomic. This module needs that
//! assumption for exactly one write per panic (simpler than Phase 53's
//! own multi-phase durability scheme, which exists because a
//! *transaction* can span many sectors - a crash-dump record fits in
//! one, so there is no analogous "torn between sectors" window to
//! protect against, only "torn within a sector," which the checksum
//! below catches and safely degrades to "no valid report" for, the
//! same as a transaction that never durably committed in Phase 53).

use core::sync::atomic::{AtomicU32, Ordering};

extern "C" {
    fn blockdev_read_sectors_absolute(lba: u32, sector_count: u8, buffer: *mut u8) -> bool;
    fn blockdev_write_sectors_absolute(lba: u32, sector_count: u8, buffer: *const u8) -> bool;
    /// See kernel/rust/journal.rs's own identical declaration's doc
    /// comment for the blockdev_id_t-as-i32 ABI note - reused
    /// verbatim, not re-argued, since this module relies on the exact
    /// same cross-FFI representation.
    fn blockdev_current() -> i32;
    fn timer_get_ticks() -> u32;
}

const SECTOR_SIZE: usize = 512;

/// Reason string capacity, including the mandatory trailing NUL this
/// module always guarantees on the decoded side (see `CrashReport`'s
/// own doc comment) - a panic message longer than this is truncated,
/// not rejected; see `rust_crashdump_write_panic`'s own doc comment.
const REASON_BYTES: usize = 64;

/// Best-effort call-stack depth - see `walk_stack`'s own doc comment
/// for what "best-effort" means here and why a hard bound exists at
/// all. Matches this project's own established "documented scope
/// limit" convention (e.g. kernel/rust/journal.rs's own
/// MAX_TXN_BLOCKS) rather than an unbounded walk.
const MAX_STACK_FRAMES: usize = 8;

/// This kernel's own identity-mapped range (see kernel/arch/x86/mm/
/// paging.c's build_identity_map(), logged at boot as "Paging enabled
/// (identity-mapped 0-64MB)") - the plausibility ceiling `walk_stack`
/// uses to decide whether a value read out of memory as "the next
/// frame's saved EBP" is even possibly a real kernel address, rather
/// than trusting a possibly-corrupted stack indefinitely.
const IDENTITY_MAP_CEILING: u32 = 64 * 1024 * 1024;

// --- On-disk record layout: magic(4) + pending(4) + checksum(4) +
// ticks(4) + reason_len(4) + reason(64) + has_regs(4) + regs(64) +
// has_fault_addr(4) + fault_addr(4) + stack_count(4) + stack(32) =
// 196 bytes - comfortably inside one 512-byte sector, with no need for
// Phase 53's own multi-sector descriptor+data split (see this file's
// header comment on why one sector is enough here). ---
const OFF_MAGIC: usize = 0;
const OFF_PENDING: usize = 4;
const OFF_CHECKSUM: usize = 8;
const OFF_TICKS: usize = 12;
const OFF_REASON_LEN: usize = 16;
const OFF_REASON: usize = 20;
const OFF_HAS_REGS: usize = OFF_REASON + REASON_BYTES; // 84
const OFF_REGS: usize = OFF_HAS_REGS + 4; // 88
const REGS_BYTES: usize = 16 * 4; // FaultRegs has 16 u32 fields
const OFF_HAS_FAULT_ADDR: usize = OFF_REGS + REGS_BYTES; // 152
const OFF_FAULT_ADDR: usize = OFF_HAS_FAULT_ADDR + 4; // 156
const OFF_STACK_COUNT: usize = OFF_FAULT_ADDR + 4; // 160
const OFF_STACK: usize = OFF_STACK_COUNT + 4; // 164
const RECORD_BYTES: usize = OFF_STACK + MAX_STACK_FRAMES * 4; // 196

/// Never produced by a freshly-zeroed disk region - see kernel/rust/
/// journal.rs's own MAGIC doc comment for the identical reasoning,
/// deliberately a different bit pattern so the two partitions could
/// never be mistaken for each other's data even if their LBAs were
/// ever accidentally swapped.
const MAGIC: u32 = 0x4E56_4344; // "NVCD"-ish, little-endian u32
/// Set in the `pending` field for a record not yet reported. Cleared
/// to 0 (not toggled to some other sentinel) once reported - see
/// `read_and_decode_at`'s own doc comment.
const PENDING_MAGIC: u32 = 0x5045_4E44; // "PEND", little-endian u32

fn put_u32_le(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn get_u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

/// FNV-1a, 32-bit - see kernel/rust/journal.rs's own identical
/// function and doc comment (non-cryptographic, torn-write detection
/// only) for the full reasoning; duplicated here rather than shared,
/// matching this project's own convention of keeping each kernel-side
/// Rust module self-contained (see e.g. sha256.rs/hmac_sha256.rs/
/// pbkdf2.rs, each already independently compiled modules rather than
/// reaching into one another's private helpers).
fn fnv1a_fold(mut h: u32, bytes: &[u8]) -> u32 {
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn compute_checksum(record: &[u8; RECORD_BYTES]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    h = fnv1a_fold(h, &record[OFF_TICKS..]); // everything after the
                                              // checksum field itself
    h
}

/// Full CPU register snapshot, laid out to match kernel/arch/x86/cpu/
/// isr.h's `registers_t` **field-for-field, in the exact same order** -
/// this is a cross-language layout contract, not something the type
/// system enforces across the FFI boundary (the same honest caveat
/// kernel/rust/journal.rs's own `blockdev_current()` doc comment makes
/// about `blockdev_id_t`): kernel/init/main.c's kernel_panic_fault()
/// passes a `const struct registers*` straight through as this type's
/// pointer, relying on both sides agreeing on this layout rather than
/// on any shared header. Every field is a plain `u32` with no padding
/// on either side (x86, 4-byte aligned throughout), so this is exactly
/// as sound as it looks, not merely hopeful.
#[repr(C)]
#[derive(Clone, Copy, PartialEq)]
pub struct FaultRegs {
    pub ds: u32,
    pub edi: u32,
    pub esi: u32,
    pub ebp: u32,
    pub esp_dummy: u32,
    pub ebx: u32,
    pub edx: u32,
    pub ecx: u32,
    pub eax: u32,
    pub int_no: u32,
    pub err_code: u32,
    pub eip: u32,
    pub cs: u32,
    pub eflags: u32,
    pub useresp: u32,
    pub ss: u32,
}

impl FaultRegs {
    const fn zeroed() -> Self {
        FaultRegs {
            ds: 0, edi: 0, esi: 0, ebp: 0, esp_dummy: 0, ebx: 0, edx: 0,
            ecx: 0, eax: 0, int_no: 0, err_code: 0, eip: 0, cs: 0,
            eflags: 0, useresp: 0, ss: 0,
        }
    }

    fn encode_into(&self, buf: &mut [u8]) {
        let fields = [
            self.ds, self.edi, self.esi, self.ebp, self.esp_dummy,
            self.ebx, self.edx, self.ecx, self.eax, self.int_no,
            self.err_code, self.eip, self.cs, self.eflags, self.useresp,
            self.ss,
        ];
        for (i, v) in fields.iter().enumerate() {
            put_u32_le(buf, i * 4, *v);
        }
    }

    fn decode_from(buf: &[u8]) -> Self {
        let f = |i: usize| get_u32_le(buf, i * 4);
        FaultRegs {
            ds: f(0), edi: f(1), esi: f(2), ebp: f(3), esp_dummy: f(4),
            ebx: f(5), edx: f(6), ecx: f(7), eax: f(8), int_no: f(9),
            err_code: f(10), eip: f(11), cs: f(12), eflags: f(13),
            useresp: f(14), ss: f(15),
        }
    }
}

/// Decoded output of `rust_crashdump_check_and_report()` - the C-side
/// mirror (kernel/fs/vfs.c's `crash_report_t`) must match this layout
/// field-for-field, the same cross-FFI contract `FaultRegs` documents
/// above. `reason` is always NUL-terminated somewhere within its
/// `REASON_BYTES`, regardless of how long the original panic message
/// was, so C can format it directly with `%s` - see
/// `rust_crashdump_write_panic`'s own doc comment on truncation.
#[repr(C)]
pub struct CrashReport {
    pub ticks: u32,
    pub reason: [u8; REASON_BYTES],
    pub has_regs: u32,
    pub regs: FaultRegs,
    pub has_fault_addr: u32,
    pub fault_addr: u32,
    pub stack: [u32; MAX_STACK_FRAMES],
    pub stack_count: u32,
}

impl CrashReport {
    const fn zeroed() -> Self {
        CrashReport {
            ticks: 0,
            reason: [0u8; REASON_BYTES],
            has_regs: 0,
            regs: FaultRegs::zeroed(),
            has_fault_addr: 0,
            fault_addr: 0,
            stack: [0u32; MAX_STACK_FRAMES],
            stack_count: 0,
        }
    }
}

struct CrashState {
    configured: bool,
    /// See `crash_region_usable`'s own doc comment - identical hazard
    /// and fix to kernel/rust/journal.rs's own `configured_device`.
    configured_device: i32,
    crash_start_lba: u32,
    crash_len_sectors: u32,
}

impl CrashState {
    const fn new() -> Self {
        CrashState {
            configured: false,
            configured_device: -1,
            crash_start_lba: 0,
            crash_len_sectors: 0,
        }
    }
}

static mut CRASH: CrashState = CrashState::new();

/// Purely for the self-test's own scratch-sector rotation (see
/// `rust_crashdump_selftest`) - identical role to kernel/rust/
/// journal.rs's own `SELFTEST_ROUND`.
static SELFTEST_ROUND: AtomicU32 = AtomicU32::new(0);

/// How far past the real record (at `crash_start_lba + 0`) the self-
/// test's own private scratch slot lives - far enough that it can
/// never collide with, or be confused for, a real crash record. See
/// kernel/rust/journal.rs's own identical `scratch_base` convention.
const SELFTEST_SCRATCH_OFFSET: u32 = 8;

/// Identical hazard, identical fix, to kernel/rust/journal.rs's own
/// `journal_region_usable()` - see that function's doc comment for the
/// full account of *why* (Phase 46's virtio-blk self-test temporarily
/// makes a different disk "active"). A crash mid-way through that
/// temporary window would otherwise durably write this module's report
/// to the wrong physical disk's absolute LBAs.
fn crash_region_usable(cs: &CrashState) -> bool {
    cs.configured && unsafe { blockdev_current() } == cs.configured_device
}

/// Best-effort walk of the x86 `cdecl` EBP frame-pointer chain,
/// starting at `start_ebp`, collecting up to `MAX_STACK_FRAMES` return
/// addresses into `out`. Relies on this kernel's own CFLAGS keeping
/// frame pointers intact end to end (see kernel/init/main.c's own
/// comment on why `-fno-omit-frame-pointer` was added specifically for
/// this) - without that flag, `-O2` may omit EBP-based frames for some
/// functions, silently truncating (never corrupting) this walk.
///
/// A real, stated risk, not glossed over: this function is reached
/// from a panic - possibly *because* of stack or memory corruption -
/// and following a frame-pointer chain is exactly "trusting memory
/// content" the panic itself may have invalidated. Two independent
/// bounds exist specifically to keep a corrupted chain from becoming a
/// second fault or an infinite loop rather than a merely-short one:
/// every candidate EBP must be 4-byte aligned and fall inside this
/// kernel's own identity-mapped range (`IDENTITY_MAP_CEILING`), and
/// each successive EBP must be strictly greater than the one before it
/// (a real call stack always grows toward *lower* addresses as it
/// deepens, so frame N+1's saved EBP - one frame *up*, toward the
/// caller - must be *higher* than frame N's; anything else means a
/// corrupt or cyclic chain). Neither bound *proves* every read here is
/// safe - a sufficiently corrupted stack could still describe a
/// plausible-looking address that isn't actually valid stack memory -
/// but see kernel_panic_fault()'s own comment for why a second fault
/// reached from here is bounded, not catastrophic, even in that case.
///
/// # Safety
/// Reads raw memory starting at `start_ebp` under the bounds described
/// above - only as safe as those bounds actually hold for the stack
/// it's given.
unsafe fn walk_stack(start_ebp: u32, out: &mut [u32; MAX_STACK_FRAMES]) -> usize {
    let mut ebp = start_ebp;
    let mut count = 0;
    while count < MAX_STACK_FRAMES {
        if ebp < 0x1000 || ebp >= IDENTITY_MAP_CEILING || ebp % 4 != 0 {
            break;
        }
        let ret_addr = core::ptr::read((ebp + 4) as *const u32);
        let next_ebp = core::ptr::read(ebp as *const u32);
        out[count] = ret_addr;
        count += 1;
        if next_ebp <= ebp {
            break; // must strictly increase each step - see doc comment
        }
        ebp = next_ebp;
    }
    count
}

fn encode_record(
    ticks: u32,
    reason: &[u8],
    has_regs: bool,
    regs: &FaultRegs,
    has_fault_addr: bool,
    fault_addr: u32,
    stack: &[u32],
    stack_count: usize,
    pending: bool,
) -> [u8; RECORD_BYTES] {
    let mut buf = [0u8; RECORD_BYTES];

    put_u32_le(&mut buf, OFF_MAGIC, MAGIC);
    put_u32_le(&mut buf, OFF_PENDING, if pending { PENDING_MAGIC } else { 0 });
    put_u32_le(&mut buf, OFF_TICKS, ticks);

    let reason_len = core::cmp::min(reason.len(), REASON_BYTES - 1);
    put_u32_le(&mut buf, OFF_REASON_LEN, reason_len as u32);
    buf[OFF_REASON..OFF_REASON + reason_len].copy_from_slice(&reason[..reason_len]);
    // buf's own initial zero-fill already NUL-terminates the rest of
    // the REASON_BYTES window, including index REASON_BYTES - 1 when
    // reason_len == REASON_BYTES - 1 exactly.

    put_u32_le(&mut buf, OFF_HAS_REGS, has_regs as u32);
    regs.encode_into(&mut buf[OFF_REGS..OFF_REGS + REGS_BYTES]);

    put_u32_le(&mut buf, OFF_HAS_FAULT_ADDR, has_fault_addr as u32);
    put_u32_le(&mut buf, OFF_FAULT_ADDR, fault_addr);

    let stack_count = core::cmp::min(stack_count, MAX_STACK_FRAMES);
    put_u32_le(&mut buf, OFF_STACK_COUNT, stack_count as u32);
    for i in 0..stack_count {
        put_u32_le(&mut buf, OFF_STACK + i * 4, stack[i]);
    }

    let checksum = compute_checksum(&buf);
    put_u32_le(&mut buf, OFF_CHECKSUM, checksum);

    buf
}

/// Writes one already-encoded record to `lba` as a single sector write
/// - see this file's header comment on why one sector is assumed
/// atomic and is enough here.
fn write_record_at(lba: u32, record: &[u8; RECORD_BYTES]) -> bool {
    let mut sector = [0u8; SECTOR_SIZE];
    sector[..RECORD_BYTES].copy_from_slice(record);
    unsafe { blockdev_write_sectors_absolute(lba, 1, sector.as_ptr()) }
}

/// Reads and validates the record at `lba`. Returns -1 on a real I/O
/// error, 0 if there is no valid, still-pending record here (never
/// written, corrupt/torn - caught by the checksum, exactly like
/// kernel/rust/journal.rs's own recovery path - or already reported by
/// an earlier boot), or 1 with `*out` (when `out` is `Some`) filled in
/// and the on-disk record rewritten with `pending` cleared so it is
/// reported at most once automatically.
fn read_and_decode_at(lba: u32, out: Option<&mut CrashReport>) -> i32 {
    let mut sector = [0u8; SECTOR_SIZE];
    if !unsafe { blockdev_read_sectors_absolute(lba, 1, sector.as_mut_ptr()) } {
        return -1;
    }

    let mut record = [0u8; RECORD_BYTES];
    record.copy_from_slice(&sector[..RECORD_BYTES]);

    let magic = get_u32_le(&record, OFF_MAGIC);
    let pending = get_u32_le(&record, OFF_PENDING);
    let stored_checksum = get_u32_le(&record, OFF_CHECKSUM);

    if magic != MAGIC || pending != PENDING_MAGIC {
        return 0; // nothing pending - the ordinary case on a clean boot
    }

    let recomputed = compute_checksum(&record);
    if recomputed != stored_checksum {
        // Torn/corrupt record - the same honest degradation as
        // kernel/rust/journal.rs's own checksum-mismatch path: report
        // nothing rather than risk showing a developer fabricated
        // register values.
        return 0;
    }

    if let Some(report) = out {
        report.ticks = get_u32_le(&record, OFF_TICKS);
        let reason_len = core::cmp::min(
            get_u32_le(&record, OFF_REASON_LEN) as usize,
            REASON_BYTES - 1,
        );
        report.reason = [0u8; REASON_BYTES];
        report.reason[..reason_len]
            .copy_from_slice(&record[OFF_REASON..OFF_REASON + reason_len]);
        report.has_regs = get_u32_le(&record, OFF_HAS_REGS);
        report.regs = FaultRegs::decode_from(&record[OFF_REGS..OFF_REGS + REGS_BYTES]);
        report.has_fault_addr = get_u32_le(&record, OFF_HAS_FAULT_ADDR);
        report.fault_addr = get_u32_le(&record, OFF_FAULT_ADDR);
        let stack_count = core::cmp::min(
            get_u32_le(&record, OFF_STACK_COUNT) as usize,
            MAX_STACK_FRAMES,
        );
        report.stack = [0u32; MAX_STACK_FRAMES];
        for i in 0..stack_count {
            report.stack[i] = get_u32_le(&record, OFF_STACK + i * 4);
        }
        report.stack_count = stack_count as u32;
    }

    // Mark consumed: rewrite with pending cleared, everything else
    // (including magic - so a future manual inspection can still see
    // "here's the last crash, already reported") left exactly as is.
    let mut cleared = record;
    put_u32_le(&mut cleared, OFF_PENDING, 0);
    // Checksum covers OFF_TICKS.. onward, which does not include
    // OFF_PENDING (see compute_checksum) - so it does not need
    // recomputing here.
    let _ = write_record_at(lba, &cleared);

    1
}

/// Configures the on-disk crash-dump region - `start_lba` is absolute
/// (this module only ever uses the `_absolute` blockdev variants, the
/// same reasoning kernel/rust/journal.rs's own `rust_journal_configure`
/// doc comment gives). Called once from kernel/fs/vfs.c's vfs_init(),
/// right next to Phase 53's own journal configure/recover call, only
/// when a disk's partition table has a fourth partition (this
/// project's own tools/build-disk-image.sh now creates one). A disk
/// with no fourth partition simply never calls this - every panic on
/// such a disk logs to serial/VGA exactly as it always has, since
/// `rust_crashdump_write_panic` below degrades to a safe no-op when
/// unconfigured.
#[no_mangle]
pub extern "C" fn rust_crashdump_configure(start_lba: u32, len_sectors: u32) {
    let cs = unsafe { &mut CRASH };
    cs.configured = len_sectors >= 1;
    cs.configured_device = unsafe { blockdev_current() };
    cs.crash_start_lba = start_lba;
    cs.crash_len_sectors = len_sectors;
}

/// The real integration point - see this file's header comment, step
/// 2-3. Called from kernel/init/main.c's kernel_panic_fault(), always,
/// for every panic (both the CPU-exception and the software-detected
/// kind - see that function's own comment on the difference).
///
/// `regs_ptr` is null for a software-detected panic (no real CPU
/// exception occurred - see `FaultRegs`'s own doc comment for the
/// layout contract when it is non-null). `walk_ebp` is always
/// provided regardless - `regs_ptr->ebp` for a real fault, or the
/// caller's own current frame pointer (`__builtin_frame_address(0)`)
/// otherwise - so a best-effort stack trace is captured either way.
///
/// A panic reason longer than `REASON_BYTES - 1` bytes is silently
/// truncated, not rejected - a crash dump missing the tail of an
/// unusually long message is far more useful than no crash dump at
/// all, the same "degrade, don't refuse" philosophy this module
/// applies to an unconfigured or too-small region.
///
/// Returns `false` if unconfigured, the configured region belongs to a
/// different (currently inactive) disk (see `crash_region_usable`), or
/// the write itself failed - kernel_panic_fault() logs this but
/// (correctly) does not treat it as a reason to change its own
/// existing halt behavior; a crash dump that couldn't be saved should
/// never prevent a real panic from still being reported the way it
/// always was.
///
/// # Safety
/// `reason_ptr` must be valid for `reason_len` readable bytes.
/// `regs_ptr`, when non-null, must point to a valid, fully-initialized
/// value laid out exactly as `FaultRegs` documents.
#[no_mangle]
pub unsafe extern "C" fn rust_crashdump_write_panic(
    reason_ptr: *const u8,
    reason_len: u32,
    regs_ptr: *const FaultRegs,
    has_fault_addr: bool,
    fault_addr: u32,
    walk_ebp: u32,
) -> bool {
    let cs = &CRASH;
    if !crash_region_usable(cs) {
        return false;
    }

    let reason = core::slice::from_raw_parts(reason_ptr, reason_len as usize);

    let (has_regs, regs) = if regs_ptr.is_null() {
        (false, FaultRegs::zeroed())
    } else {
        (true, *regs_ptr)
    };

    let mut stack = [0u32; MAX_STACK_FRAMES];
    let stack_count = walk_stack(walk_ebp, &mut stack);

    let ticks = timer_get_ticks();

    let record = encode_record(
        ticks, reason, has_regs, &regs, has_fault_addr, fault_addr, &stack,
        stack_count, true,
    );
    write_record_at(cs.crash_start_lba, &record)
}

/// Real boot-time check - see this file's header comment, step 4.
/// Called once from kernel/fs/vfs.c's vfs_init(). Safe (one 1-sector
/// read, usually followed by nothing else) to call even when nothing
/// is pending, the ordinary case on every clean shutdown.
///
/// # Safety
/// `out` must point to a valid, writable `CrashReport`.
#[no_mangle]
pub unsafe extern "C" fn rust_crashdump_check_and_report(out: *mut CrashReport) -> i32 {
    let cs = &CRASH;
    if !crash_region_usable(cs) {
        return -1;
    }
    let report = &mut *out;
    read_and_decode_at(cs.crash_start_lba, Some(report))
}

/// Ring-0 self-test, called from kernel_main() the same way every
/// other kernel-side Rust module in this project proves itself (see
/// e.g. kernel/rust/journal.rs's own rust_journal_selftest()).
/// Exercises the encode/decode/checksum/"report once" logic directly
/// against the real, already-configured crash-dump region, using a
/// dedicated scratch sector (`SELFTEST_SCRATCH_OFFSET` past the real
/// slot - see that constant's own doc comment) so this self-test can
/// never collide with, or be confused for, a real crash record:
///
///   Part A - a full record (real register snapshot, a faulting
///   address, and a synthetic stack trace) is written and then read
///   back through the exact same `read_and_decode_at` the real boot
///   path uses, confirming it reports "found" (1) and every single
///   field - ticks, reason, every register, the faulting address, and
///   the whole stack trace - decodes back byte-for-byte identical to
///   what was written.
///
///   Part B - the *same* scratch slot is read again, immediately,
///   confirming it now reports "nothing pending" (0) - proving the
///   "reported at most once" mechanism (the on-disk `pending` field
///   being cleared by Part A's own read) actually works, not just that
///   the write/read path works in isolation.
///
///   Part C - a *second* record, this time with `has_regs = false`
///   (the software-detected-panic shape, e.g. kernel/config/
///   userscfg.c's own size-agreement check) is written and read back,
///   confirming `has_regs` correctly decodes as false and the reason/
///   stack fields still round-trip correctly without a register
///   snapshot present - a distinct code path from Part A's, worth
///   verifying separately rather than assumed to work because Part A
///   did.
///
/// Returns a bitmask (0 = every check passed, matching this project's
/// established self-test convention), or a negative value if no real
/// crash-dump region is configured (-1) or it's too small to safely
/// carve out this self-test's own scratch sector without risking the
/// real record slot (-2) - see kernel/init/main.c's own call site for
/// how both are handled (matching kernel/rust/journal.rs's own -1/-2
/// convention exactly).
#[no_mangle]
pub extern "C" fn rust_crashdump_selftest() -> i32 {
    let cs = unsafe { &CRASH };
    if !crash_region_usable(cs) {
        return -1;
    }
    if cs.crash_len_sectors < SELFTEST_SCRATCH_OFFSET + 4 {
        return -2;
    }

    let round = SELFTEST_ROUND.fetch_add(1, Ordering::Relaxed) % 4;
    let scratch = cs
        .crash_start_lba
        .wrapping_add(SELFTEST_SCRATCH_OFFSET)
        .wrapping_add(round);

    let mut code = 0i32;

    // --- Part A: full record, round-trip every field ---
    let regs_a = FaultRegs {
        ds: 0x11, edi: 0x2222_2222, esi: 0x3333_3333, ebp: 0x4444_4444,
        esp_dummy: 0x5555_5555, ebx: 0x6666_6666, edx: 0x7777_7777,
        ecx: 0x8888_8888, eax: 0x9999_9999, int_no: 14, err_code: 0x4,
        eip: 0xAAAA_AAAA, cs: 0x08, eflags: 0x0202,
        useresp: 0xBBBB_BBBB, ss: 0x10,
    };
    let reason_a = b"selftest: synthetic fault with registers";
    let stack_a: [u32; MAX_STACK_FRAMES] =
        [0x1000, 0x2000, 0x3000, 0x4000, 0x5000, 0x6000, 0x7000, 0x8000];

    let record_a = encode_record(
        0xC0FF_EE, reason_a, true, &regs_a, true, 0xDEAD_BEEF, &stack_a,
        MAX_STACK_FRAMES, true,
    );
    let wrote_a = write_record_at(scratch, &record_a);

    let mut report_a = CrashReport::zeroed();
    let found_a = wrote_a && read_and_decode_at(scratch, Some(&mut report_a)) == 1;

    let part_a_ok = found_a
        && report_a.ticks == 0xC0FF_EE
        && &report_a.reason[..reason_a.len()] == reason_a
        && report_a.has_regs == 1
        && report_a.regs == regs_a
        && report_a.has_fault_addr == 1
        && report_a.fault_addr == 0xDEAD_BEEF
        && report_a.stack_count as usize == MAX_STACK_FRAMES
        && report_a.stack == stack_a;
    if !part_a_ok {
        code |= 1;
    }

    // --- Part B: same slot, must now report "nothing pending" ---
    let mut report_b = CrashReport::zeroed();
    let recheck = read_and_decode_at(scratch, Some(&mut report_b));
    if recheck != 0 {
        code |= 2;
    }

    // --- Part C: a software-detected-style record, no registers ---
    let reason_c = b"selftest: synthetic panic, no registers";
    let stack_c: [u32; MAX_STACK_FRAMES] = [0x9000, 0xA000, 0, 0, 0, 0, 0, 0];
    let record_c = encode_record(
        0x1234, reason_c, false, &FaultRegs::zeroed(), false, 0, &stack_c,
        2, true,
    );
    let wrote_c = write_record_at(scratch, &record_c);

    let mut report_c = CrashReport::zeroed();
    let found_c = wrote_c && read_and_decode_at(scratch, Some(&mut report_c)) == 1;

    let part_c_ok = found_c
        && &report_c.reason[..reason_c.len()] == reason_c
        && report_c.has_regs == 0
        && report_c.has_fault_addr == 0
        && report_c.stack_count == 2
        && report_c.stack[0] == 0x9000
        && report_c.stack[1] == 0xA000;
    if !part_c_ok {
        code |= 4;
    }

    code
}
