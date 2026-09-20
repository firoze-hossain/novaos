//! kernel/rust/acpi.rs - Phase 44: ACPI MADT parsing - CPU topology
//! discovery, the genuine first prerequisite for SMP.
//!
//! Scope, deliberately, and stated as directly as possible: this is
//! NOT SMP support. This kernel remains single-core after this phase
//! - nothing here boots a second CPU, sets up a Local/IO-APIC in
//! place of the existing 8259 PIC, or adds a single lock anywhere in
//! this kernel's existing, working code. What this phase actually
//! does: discover *how many* CPUs a machine has and their APIC IDs -
//! information every one of the real, much larger prerequisites still
//! ahead (a Local APIC driver, an AP bootstrap trampoline in low
//! memory, per-CPU data structures, and - the genuinely hard part,
//! per this project's own release-readiness roadmap - making every
//! existing shared kernel data structure actually safe for more than
//! one CPU to touch at once) needs before any of them can even begin.
//! Read-only, informational, and entirely independent of this
//! kernel's existing interrupt/scheduling code, specifically so
//! adding it carries none of the real risk full SMP work would -
//! nothing about how this kernel currently boots or runs changes.
//!
//! Why this is the right first step, not an arbitrary one: every real
//! OS (Linux, Windows, macOS) parses exactly this information, this
//! way, before doing anything else SMP-related - you cannot send an
//! INIT-SIPI-SIPI sequence to wake a second CPU, or set up per-CPU
//! state for it, without first knowing it exists and what its APIC ID
//! is.
//!
//! Format note: implements the ACPI 1.0 RSDP + RSDT path (32-bit
//! table pointers) rather than also handling the ACPI 2.0+ XSDT
//! (64-bit pointers) - QEMU's default firmware provides both, and
//! this kernel is itself 32-bit throughout, so the simpler, sufficient
//! path was chosen deliberately, not because the newer one wasn't
//! considered.
//!
//! A real, honest limitation, found and precisely diagnosed rather
//! than left as a mysterious "sometimes doesn't work" - fixed since,
//! see the Phase 56 paragraph below, but kept here for the full
//! account: this module could only safely read memory within this
//! kernel's own static identity-mapped range (paging.c's own
//! confirmed 0-64MB - see IDENTITY_MAPPED_LIMIT below), and on this
//! project's own default `-m 512M` test config, QEMU places the
//! actual RSDT/MADT tables well above that boundary - discovery
//! correctly, gracefully reported "not found" in that case (proven
//! safe: an earlier version of this module read those tables'
//! signature/length *before* checking bounds at all, and crashed with
//! a real page fault the first time it ran against real hardware
//! instead of the self-test's own synthetic, stack-allocated table).
//! Confirmed this was a memory-size boundary issue, not a parsing bug,
//! by testing with `-m 32M` (comfortably under 64MB) instead: real
//! discovery then succeeded, and the reported CPU count was
//! independently verified against three different `-smp N` values
//! (default/1, 2, and 4), each matching exactly - see PROGRESS.md's
//! Phase 44 entry for the full account.
//!
//! Phase 56 closes this gap - not by extending paging.c's static
//! boot-time identity map (still exactly 0-64MB, untouched), but by
//! mapping whatever specific page a read actually needs, on demand,
//! via the same `paging_map_page()` every per-process address space
//! in this kernel already uses. See `ensure_mapped()`'s own comment
//! below for the full reasoning, including why Phase 56 needed this
//! capability anyway, unconditionally, for a completely different
//! reason (kernel/rust/apic.rs's own LAPIC/IO-APIC MMIO access, at
//! fixed hardware addresses nowhere near even a generous identity
//! map), which is what made finally closing Phase 44's own
//! documented gap essentially free rather than its own separate
//! project. Real ACPI discovery under this project's standard
//! `-m 512M` test config - previously a `[WARN] ... not found`,
//! silently untested by `make test` - now succeeds deterministically,
//! and PROGRESS.md's Phase 56 entry documents this as a real, if
//! secondary, improvement in this project's own test coverage.
//!
//! Phase 55 extends this same module - deliberately, not as a new
//! file - to close the release-readiness list's own "real shutdown,
//! not just reboot" row, which that document itself pointed straight
//! back here: "the foundation (real, verified ACPI table parsing) is
//! no longer the blocker... the specific shutdown piece is still
//! unbuilt." Real ACPI power-off (the "S5" sleep state) needs a
//! second table this module didn't read before - the FADT ("Fixed
//! ACPI Description Table," real signature `FACP`, not `FADT` -
//! confirmed against the ACPI spec, not assumed) - plus one thing no
//! table field alone provides: the two-byte `SLP_TYPa`/`SLP_TYPb`
//! values that tell the real chipset *which* sleep state S5 actually
//! is on this specific machine, which only exist encoded inside the
//! DSDT's own AML (ACPI Machine Language) bytecode, under a
//! `Name (_S5, Package (...) { ... })` object. This module does not
//! become a general AML interpreter to get those two bytes - real AML
//! interpretation is a huge, separate undertaking (a byte-code VM
//! with its own operand stack, name resolution, and dozens of
//! opcodes) no hobby kernel needs in full just to shut down. Instead
//! it uses the same narrow, well-precedented technique real hobby
//! OSes (and this exact scan pattern, independently documented on
//! OSDev.org's own "Shutdown" page) have used for years: scan the
//! DSDT's raw bytes for the literal 4-byte ASCII name `_S5_`, then
//! decode just the few bytes that immediately follow it directly -
//! the `PackageOp`, a `PkgLength` whose encoding is skipped rather
//! than needing to be computed, an element count, and the two
//! `SLP_TYP` integers themselves - without interpreting any other AML
//! in the table at all. `find_s5_sleep_type()`'s own comment below has
//! the byte-for-byte reasoning; PROGRESS.md's Phase 55 entry has the
//! full account of why this narrow slice is both correct and enough.

const MAX_CPUS: usize = 16; // a reasonable, bounded cap - matching this
                             // kernel's own established preference for
                             // fixed-size arrays over dynamic allocation
                             // (MAX_PROCESSES, MAX_OPEN_FILES, MAX_PIPES
                             // all follow the same pattern)

#[repr(C)]
pub struct AcpiCpuDiscovery {
    pub found_acpi: bool,
    pub found_madt: bool,
    pub cpu_count: u32,
    pub apic_ids: [u8; MAX_CPUS],
    pub local_apic_phys: u32,
}

impl AcpiCpuDiscovery {
    const fn empty() -> Self {
        AcpiCpuDiscovery {
            found_acpi: false,
            found_madt: false,
            cpu_count: 0,
            apic_ids: [0; MAX_CPUS],
            local_apic_phys: 0,
        }
    }
}

/// Phase 56: exactly the same FFI hazard as `AcpiShutdownInfo` below
/// (see its own doc comment for the full account, including how it
/// was found) - discovered *in this struct too*, retroactively, while
/// building Phase 56 on top of it. `AcpiCpuDiscovery` above keeps its
/// two `bool` fields exactly as Phase 44 shipped them (nothing reads
/// that struct's own `found_acpi`/`found_madt` fields across the FFI
/// boundary directly - only individual, separately-typed out-params
/// do, see `rust_acpi_discover_cpus()` just below), but this kernel's
/// call site (kernel/init/main.c) declared those out-params - and,
/// worse, this function's own *return value* - using C `bool` too.
/// A return value doesn't have the same "fields shift by N bytes"
/// failure mode a struct field does (there's only one value, not a
/// layout), but it has its own real problem: this target's calling
/// convention only guarantees the low 8 bits (AL) of EAX are
/// meaningful for a `bool`-returning function: this kernel's own
/// C `bool` is a 4-byte enum, so `bool found = rust_acpi_discover_cpus(...)`
/// on the C side reads *all 32 bits* of EAX as the return value - if
/// the upper 24 bits happen to be nonzero (unspecified, not
/// guaranteed zero), `found` can read as true even when this function
/// returned Rust `false` (AL = 0). Fixed the same way
/// `AcpiShutdownInfo` was: this function's return value and its
/// `out_found_acpi` parameter are both `u8` now, not `bool`, on
/// either side of the boundary - see the updated extern declaration
/// and call site in kernel/init/main.c.

extern "C" {
    /// C's own `bool` return, deliberately received here as a full
    /// `i32` rather than Rust `bool` - see `AcpiCpuDiscovery`'s own
    /// doc comment above for exactly why a truncated (AL-only) read
    /// of a C-`bool`-returning function is a real, found hazard on
    /// this target, not a theoretical one. `page_directory_ptr` must
    /// be a directly-dereferenceable pointer, not just any physical
    /// address (see paging.h's own doc comment on `paging_map_page`)
    /// - `paging_kernel_directory_phys()`'s result satisfies that
    /// because it is, itself, always within the static 0-64MB
    /// identity map (it is a `static` array inside the kernel's own
    /// loaded image, which starts at 1MB - see tools/linker.ld).
    fn paging_map_page(page_directory_ptr: *mut u32, virt_addr: u32,
                        phys_addr: u32, flags: u32) -> i32;
    fn paging_kernel_directory_phys() -> u32;
}

const PAGE_PRESENT: u32 = 0x1;
const PAGE_WRITE: u32 = 0x2;

/// Reads a `u8` from a raw physical address. Safe to call on *any*
/// address below 4GB (not just the static 0-64MB identity map) as of
/// Phase 56 - see `ensure_mapped()` below for what changed and why.
///
/// # Safety
/// `addr` must be a physical address this module has already decided
/// is safe to treat as plain memory - every call site below either
/// uses a fixed, known-safe address or one bounds-checked (and, if
/// necessary, mapped) via `addr_range_safe()`/`ensure_mapped()` first.
#[inline(always)]
unsafe fn read_u8(addr: u32) -> u8 {
    core::ptr::read_volatile(addr as *const u8)
}

#[inline(always)]
unsafe fn read_u16(addr: u32) -> u16 {
    core::ptr::read_unaligned(addr as *const u16)
}

#[inline(always)]
unsafe fn read_u32(addr: u32) -> u32 {
    core::ptr::read_unaligned(addr as *const u32)
}

/// This kernel's own *static, boot-time* identity-mapped range (see
/// paging.c's own log message, confirmed directly before ever relying
/// on it: "Paging enabled (identity-mapped 0-64MB)"). Below this
/// limit, every physical address is already mapped and safe to read
/// with no further work. At or above it, `ensure_mapped()` (added in
/// Phase 56, see its own comment) maps the specific page on demand
/// instead of refusing the read outright, which is what this module
/// did through the end of Phase 55.
const IDENTITY_MAPPED_LIMIT: u32 = 0x4000000; // 64MB

/// Phase 56: maps physical address `addr`'s containing 4KB page,
/// identity (virt == phys, matching every other mapping this kernel's
/// low memory already uses), into the *kernel's own* page directory -
/// a real, if narrow, fix for a gap Phase 44 shipped knowingly and
/// documented rather than hid: that module's own header comment
/// explained that real ACPI tables routinely live above the static
/// 64MB identity map on this project's own default `-m 512M` test
/// config, and discovery there correctly, gracefully reported "not
/// found" rather than crash - but still couldn't actually read them.
/// Phase 44's own comment named the fix ("extending this kernel's
/// identity map to cover more of physical memory") and named why it
/// wasn't attempted then: "real, separate, larger-blast-radius work
/// (touching paging.c, not this module)."
///
/// This is deliberately NOT that fix. It does not touch paging.c's
/// static boot-time identity map at all (still exactly 0-64MB, still
/// built the same way, still every other subsystem's unchanged
/// assumption). Instead it reuses paging.c's own existing, already-
/// shipped `paging_map_page()` - the same function every per-process
/// address space in this kernel already uses - to map exactly the one
/// page a given read needs, into the kernel's own directory, the
/// moment this module needs it. The blast radius is exactly this
/// module (and, from Phase 56 on, kernel/rust/apic.rs's own LAPIC/
/// IO-APIC MMIO access, which needs this unconditionally: their fixed
/// hardware MMIO addresses - conventionally 0xFEE00000/0xFEC00000,
/// just under the 4GB mark - sit *nowhere near* even a generously
/// large static identity map, on any `-m` size).
///
/// Idempotent and cheap to call repeatedly on an already-mapped page
/// (`paging_map_page()` just overwrites the same page-table entry
/// with the same value; no allocation happens unless a *new* page
/// table is needed for a 4MB region nothing in this range has touched
/// yet) - deliberately not cached or short-circuited here, since every
/// call site is one-time boot discovery over at most a handful of
/// small ACPI tables, not a hot path. kernel/rust/apic.rs's own LAPIC
/// EOI/IPI register access, which *is* a hot path (once per hardware
/// interrupt), maps its MMIO page exactly once at init time instead
/// of going through this function - see that module's own comment.
pub(crate) fn ensure_mapped(addr: u32) {
    if addr < IDENTITY_MAPPED_LIMIT {
        return; // already covered by the static boot-time map
    }
    let page = addr & !0xFFFu32;
    unsafe {
        let kernel_dir = paging_kernel_directory_phys() as *mut u32;
        paging_map_page(kernel_dir, page, page, PAGE_PRESENT | PAGE_WRITE);
    }
    // No failure path: if this is ever out of physical memory to
    // allocate a new page table frame from, the *read* that follows
    // will simply fault - an honest failure (a real page fault, this
    // kernel's own diagnosable panic path - see paging.c's
    // page_fault_handler()) rather than a silently wrong value. This
    // module has no way to abort a read already in progress partway
    // through a multi-byte scan, the same reasoning `addr_range_safe`
    // already documented for `checked_add` above.
}

/// Must be checked before any read at `addr` - true iff every byte in
/// `[addr, addr+len)` is safe to read, mapping it on demand first if
/// it falls outside the static identity range (see `ensure_mapped()`).
/// Uses `checked_add` deliberately, not plain `+`, so a pathological
/// `len` large enough to overflow `u32` arithmetic is rejected rather
/// than wrapping into a false "safe" result. The only remaining
/// rejection is that overflow case and `len == 0` - Phase 56 removed
/// the fixed 64MB ceiling this function enforced through Phase 55 (see
/// `ensure_mapped()`'s own comment for what replaced it and why doing
/// so is safe).
fn addr_range_safe(addr: u32, len: u32) -> bool {
    let end = match addr.checked_add(len) {
        Some(e) => e,
        None => return false,
    };
    if len == 0 {
        return false;
    }
    let mut page = addr & !0xFFFu32;
    while page < end {
        ensure_mapped(page);
        page = match page.checked_add(0x1000) {
            Some(p) => p,
            None => break, // page was already within 4KB of u32::MAX -
                            // nothing legitimate lives there; stop
                            // rather than risk wrapping back to 0 and
                            // looping forever
        };
    }
    true
}

fn checksum_ok(addr: u32, len: u32) -> bool {
    if !addr_range_safe(addr, len) {
        return false;
    }
    let mut sum: u8 = 0;
    let mut i = 0u32;
    while i < len {
        sum = sum.wrapping_add(unsafe { read_u8(addr + i) });
        i += 1;
    }
    sum == 0
}

fn signature_matches(addr: u32, expected: &[u8; 4]) -> bool {
    if !addr_range_safe(addr, 4) {
        return false;
    }
    for i in 0..4u32 {
        if unsafe { read_u8(addr + i) } != expected[i as usize] {
            return false;
        }
    }
    true
}

/// Scans a physical address range on 16-byte boundaries for the
/// 8-byte "RSD PTR " signature (note the trailing space - part of the
/// real signature, not a formatting artifact) with a valid ACPI 1.0
/// checksum (the first 20 bytes of the structure summing to 0 mod
/// 256) - the standard search this exact way every real OS uses,
/// checksum required specifically because the signature alone is not
/// rare enough in arbitrary firmware memory to trust without it.
fn scan_for_rsdp(start: u32, end: u32) -> Option<u32> {
    const SIG: &[u8; 8] = b"RSD PTR ";
    let mut addr = start & !0xF;
    while addr < end {
        let mut matches = true;
        for i in 0..8u32 {
            if unsafe { read_u8(addr + i) } != SIG[i as usize] {
                matches = false;
                break;
            }
        }
        if matches && checksum_ok(addr, 20) {
            return Some(addr);
        }
        addr += 16;
    }
    None
}

fn find_rsdp() -> Option<u32> {
    // The EBDA segment pointer lives at a fixed, well-known physical
    // address (0x40E, a real-mode-era convention still honored by
    // every BIOS/UEFI-CSM this kernel targets) - a 16-bit real-mode
    // segment value, so the actual physical address is that value
    // shifted left 4 bits.
    let ebda_segment = unsafe { read_u16(0x40E) };
    let ebda_addr = (ebda_segment as u32) << 4;
    if ebda_addr != 0 {
        if let Some(found) = scan_for_rsdp(ebda_addr, ebda_addr + 1024) {
            return Some(found);
        }
    }
    scan_for_rsdp(0xE0000, 0x100000)
}

fn find_madt(rsdt_addr: u32) -> Option<u32> {
    // Validate the full, fixed-size SDT header range upfront - every
    // ACPI table's header is exactly 36 bytes, always, regardless of
    // what this specific table's own `length` field later says.
    // Reading the signature and length below is only safe once this
    // has passed; checking bounds per-field instead (as an earlier,
    // buggy version of this function did) leaves a gap between what
    // one field's own check covers and where the next field's read
    // actually lands.
    if !addr_range_safe(rsdt_addr, 36) {
        return None;
    }
    if !signature_matches(rsdt_addr, b"RSDT") {
        return None;
    }
    let length = unsafe { read_u32(rsdt_addr + 4) };
    if length < 36 || !checksum_ok(rsdt_addr, length) {
        return None;
    }

    let entry_count = (length - 36) / 4;
    for i in 0..entry_count {
        let entry_addr = rsdt_addr + 36 + i * 4;
        if !addr_range_safe(entry_addr, 4) {
            continue; // a malformed/out-of-range entry pointer is
                      // skipped, not trusted - the rest of the RSDT
                      // may still contain a valid MADT pointer
        }
        let table_addr = unsafe { read_u32(entry_addr) };
        if signature_matches(table_addr, b"APIC") {
            return Some(table_addr);
        }
    }
    None
}

fn parse_madt(madt_addr: u32, out: &mut AcpiCpuDiscovery) {
    // Same reasoning as find_madt() above: validate the full,
    // fixed-size region this function unconditionally reads from
    // (SDT header (36) + LocalApicAddress (4) + Flags (4) = 44 bytes)
    // upfront, before any read, not field-by-field.
    if !addr_range_safe(madt_addr, 44) {
        return;
    }
    if !signature_matches(madt_addr, b"APIC") {
        return;
    }
    let length = unsafe { read_u32(madt_addr + 4) };
    if length < 44 || !checksum_ok(madt_addr, length) {
        return;
    }
    out.found_madt = true;
    out.local_apic_phys = unsafe { read_u32(madt_addr + 36) };

    let end = madt_addr + length;
    let mut cursor = madt_addr + 44; // SDT header (36) + LocalApicAddress
                                      // (4) + Flags (4)

    while cursor + 2 <= end {
        if !addr_range_safe(cursor, 2) {
            break; // can't even safely read this entry's own type/
                   // length fields - stop rather than guess
        }
        let entry_type = unsafe { read_u8(cursor) };
        let entry_len = unsafe { read_u8(cursor + 1) };
        if entry_len == 0 {
            break; // malformed - refuse to loop forever on bad data
                   // rather than trust it
        }
        if entry_type == 0 && entry_len >= 8 && cursor + 8 <= end
            && addr_range_safe(cursor, 8)
        {
            // Processor Local APIC entry (ACPI spec table 5-26):
            // type(1) length(1) acpi_processor_id(1) apic_id(1)
            // flags(4) - flags bit 0 is "Enabled".
            let apic_id = unsafe { read_u8(cursor + 3) };
            let flags = unsafe { read_u32(cursor + 4) };
            let enabled = (flags & 1) != 0;
            if enabled && (out.cpu_count as usize) < MAX_CPUS {
                out.apic_ids[out.cpu_count as usize] = apic_id;
                out.cpu_count += 1;
            }
        }
        cursor += entry_len as u32;
    }
}

fn discover() -> AcpiCpuDiscovery {
    let mut result = AcpiCpuDiscovery::empty();

    let rsdp_addr = match find_rsdp() {
        Some(addr) => addr,
        None => return result, // no ACPI - honest, graceful "found
                                // nothing," not an error; callers
                                // treat this as "assume single-core"
    };
    result.found_acpi = true;

    let rsdt_addr = unsafe { read_u32(rsdp_addr + 16) };
    if let Some(madt_addr) = find_madt(rsdt_addr) {
        parse_madt(madt_addr, &mut result);
    }

    result
}

/// Called once from kernel_main() - runs the real discovery against
/// whatever ACPI tables this machine (QEMU, in this project's own
/// test environment) actually provides, and logs the result. See
/// kernel/init/main.c's own call site for the exact log line this
/// project's test suite checks, and PROGRESS.md's Phase 44 entry for
/// how this was verified against several different `-smp N` values,
/// not just once.
#[no_mangle]
pub extern "C" fn rust_acpi_discover_cpus(out_count: *mut u32,
                                           out_local_apic_phys: *mut u32,
                                           out_found_acpi: *mut u8)
                                           -> u8 {
    let result = discover();
    unsafe {
        *out_count = result.cpu_count;
        *out_local_apic_phys = result.local_apic_phys;
        *out_found_acpi = if result.found_acpi { 1 } else { 0 };
    }
    if result.found_madt { 1 } else { 0 }
}

/// Ring-0 self-test, called directly from kernel_main() - verifies
/// the *parsing logic* in isolation, against a small, fully
/// synthetic, hand-constructed ACPI table this test controls byte-
/// for-byte, entirely independent of whatever real table QEMU
/// happens to provide. Chosen specifically because, unlike e.g.
/// kernel/rust/virtio_blk.rs's own layout self-test (which could
/// check computed offsets against independently hand-computed
/// values), there is no equivalent "known correct answer" for a real
/// machine's own ACPI tables to check parse_madt() against in
/// isolation - a synthetic table this test builds and checksums
/// itself is the only way to verify the parsing logic on its own
/// terms, separately from whether real hardware discovery (checked
/// separately - see PROGRESS.md) also works.
#[no_mangle]
pub extern "C" fn rust_acpi_selftest() -> i32 {
    let mut code = 0;

    // A synthetic MADT: header (36 bytes) + LocalApicAddress (4) +
    // Flags (4) + two Processor Local APIC entries (8 bytes each, one
    // enabled with APIC ID 0, one disabled with APIC ID 1 - proving
    // the "Enabled" flag is actually honored, not just that entries
    // are counted) + one entry of an unrelated type (1, "I/O APIC")
    // that must be skipped, not miscounted as a CPU.
    let mut table = [0u8; 36 + 4 + 4 + 8 + 8 + 8];
    // SDT header
    table[0..4].copy_from_slice(b"APIC");
    let total_len = table.len() as u32;
    table[4..8].copy_from_slice(&total_len.to_le_bytes());
    // LocalApicAddress = 0xFEE00000 (the real, standard value - not
    // load-bearing for this test, just realistic)
    table[36..40].copy_from_slice(&0xFEE00000u32.to_le_bytes());
    // Flags = 0
    table[40..44].copy_from_slice(&0u32.to_le_bytes());
    // Entry 1: Processor Local APIC, enabled, APIC ID 0
    table[44] = 0; // type
    table[45] = 8; // length
    table[46] = 0; // acpi_processor_id
    table[47] = 0; // apic_id
    table[48..52].copy_from_slice(&1u32.to_le_bytes()); // flags: enabled
    // Entry 2: an I/O APIC entry (type 1) - must be skipped
    table[52] = 1; // type
    table[53] = 8; // length (arbitrary, matches this entry's slot size)
    // Entry 3: Processor Local APIC, DISABLED, APIC ID 2 - must NOT
    // be counted
    table[60] = 0;
    table[61] = 8;
    table[62] = 0;
    table[63] = 2;
    table[64..68].copy_from_slice(&0u32.to_le_bytes()); // flags: disabled

    // Fix up the checksum last, over the whole table, the same "sum
    // of all bytes including this one must be 0 mod 256" rule
    // checksum_ok() itself checks.
    let addr = table.as_ptr() as u32;
    let sum_without_checksum_byte: u8 = table
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != 9) // checksum byte itself, SDT offset 9
        .fold(0u8, |acc, (_, &b)| acc.wrapping_add(b));
    table[9] = (0u8).wrapping_sub(sum_without_checksum_byte);

    if !checksum_ok(addr, total_len) {
        code |= 1;
    }

    let mut result = AcpiCpuDiscovery::empty();
    parse_madt(addr, &mut result);

    if !result.found_madt {
        code |= 2;
    }
    if result.local_apic_phys != 0xFEE00000 {
        code |= 4;
    }
    if result.cpu_count != 1 {
        // exactly the one *enabled* CPU - the disabled one and the
        // I/O APIC entry must both be excluded
        code |= 8;
    }
    if result.cpu_count >= 1 && result.apic_ids[0] != 0 {
        code |= 16;
    }

    code
}

// ============================================================
// Phase 55: FADT parsing + real ACPI shutdown (S5)
// ============================================================

/// PM1x_CNT register bit 0 (ACPI spec): 1 once this machine's ACPI
/// subsystem has actually been switched on. Firmware boots most real
/// machines and QEMU/SeaBIOS with this already set (Phase 44's own
/// MADT discovery succeeding at all already proves *some* ACPI state
/// exists), but it's not something to assume - `rust_acpi_shutdown()`
/// below checks it directly and only runs the real enable sequence
/// (SMI_CMD/ACPI_ENABLE) if it's actually needed.
const PM1_SCI_EN: u16 = 0x0001;

/// PM1x_CNT register bit 13 (ACPI spec) - "enter the sleep state named
/// by the SLP_TYP field (bits 10-12) now." This is the one bit that
/// actually does something; everything else in this section exists to
/// find the right port and the right 3-bit value to put next to it.
const PM1_SLP_EN: u16 = 0x2000;

/// A fixed, non-timer-based iteration count used to bound two
/// separate waits below (the ACPI-enable handshake, and the "did the
/// power-off actually happen" check after writing PM1_CNT).
/// Deliberately NOT derived from `timer_get_ticks()` the way
/// kernel/rust/journal.rs/crashdump.rs's own bounded operations are
/// free to be: this function can be reached from a ring-3 SYS_SHUTDOWN
/// syscall (kernel/arch/x86/cpu/syscall.c), and this kernel's own
/// syscall gate (syscall_stub.asm) disables interrupts for a syscall's
/// entire duration - PROGRESS.md's Phase 54 entry documents finding
/// this exact class of bug (a blocking wait that needs the PIT's IRQ0
/// to advance a tick counter, issued from inside a window where
/// interrupts can never fire, deadlocks forever). A plain busy-loop
/// iteration count sidesteps that failure mode entirely, at the
/// honestly-stated cost of being calibrated to "a lot of loop
/// iterations on whatever CPU this runs on," not to any real
/// wall-clock duration.
const BUSY_WAIT_ITERATIONS: u32 = 20_000_000;

/// `found_acpi`/`found_fadt`/`found_s5`/`sci_en_already_set` are `u8`
/// (0/1), deliberately NOT Rust `bool`, even though Rust guarantees
/// `bool` is exactly one byte in a `#[repr(C)]` struct - because the
/// *C side* of this exact boundary cannot make the same guarantee.
/// This kernel's own `bool` (kernel/include/types.h:
/// `typedef enum { false = 0, true = 1 } bool;`) is a plain C `enum`,
/// which GCC on this target sizes as a 4-byte `int` by default, not
/// one byte - a real, found-by-building-and-booting-this-exact-struct
/// mismatch, not a theoretical one (see PROGRESS.md's Phase 55 entry
/// for the full account of how this was caught: a C-side mirror
/// struct built with this kernel's own `bool` silently shifted every
/// field after the first few by 9 bytes). `kernel/fs/vfs.c`'s own
/// `crash_report_t` (Phase 54) already established the fix this
/// module follows - plain fixed-width integers for every FFI-crossing
/// struct field, never this kernel's own `bool` and never Rust's,
/// so neither side's own bool representation is ever load-bearing.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AcpiShutdownInfo {
    pub found_acpi: u8,
    pub found_fadt: u8,
    pub found_s5: u8,
    pub pm1a_cnt_blk: u32,
    pub pm1b_cnt_blk: u32,
    pub smi_cmd: u32,
    pub acpi_enable: u8,
    pub sci_en_already_set: u8,
    pub slp_typa: u16,
    pub slp_typb: u16,
}

impl AcpiShutdownInfo {
    const fn empty() -> Self {
        AcpiShutdownInfo {
            found_acpi: 0,
            found_fadt: 0,
            found_s5: 0,
            pm1a_cnt_blk: 0,
            pm1b_cnt_blk: 0,
            smi_cmd: 0,
            acpi_enable: 0,
            sci_en_already_set: 0,
            slp_typa: 0,
            slp_typb: 0,
        }
    }
}

/// Only the FADT fields this module actually uses - real FADTs have
/// several dozen more (power-button handling, C-state latencies, ACPI
/// 2.0+'s 64-bit "X_" extended-address twins of most fields below,
/// and much more), every one of them deliberately left unread. Same
/// scope discipline this module's own MADT parsing already applies
/// (ACPI 1.0's 32-bit table pointers, not also the 2.0+ XSDT) and the
/// same reasoning: QEMU's default firmware (SeaBIOS) always populates
/// the legacy 32-bit I/O-port fields below, which is what this
/// kernel's own 32-bit `in`/`out` port instructions need anyway - the
/// 2.0+ "X_" GAS-encoded twins exist for machines whose PM1 control
/// register isn't plain I/O-port space at all (rare, and not
/// expressible with a bare `outw`/`inw` regardless), not a gap this
/// phase left open by mistake.
struct FadtFields {
    dsdt_addr: u32,
    smi_cmd: u32,
    acpi_enable: u8,
    pm1a_cnt_blk: u32,
    pm1b_cnt_blk: u32,
}

/// Same structure as `find_madt()` above, real signature `FACP`
/// ("Fixed ACPI Control Panel," the historical name behind the
/// acronym mismatch with "FADT" - confirmed against the ACPI spec,
/// not assumed or guessed from the table's common name).
fn find_fadt(rsdt_addr: u32) -> Option<u32> {
    if !addr_range_safe(rsdt_addr, 36) {
        return None;
    }
    if !signature_matches(rsdt_addr, b"RSDT") {
        return None;
    }
    let length = unsafe { read_u32(rsdt_addr + 4) };
    if length < 36 || !checksum_ok(rsdt_addr, length) {
        return None;
    }

    let entry_count = (length - 36) / 4;
    for i in 0..entry_count {
        let entry_addr = rsdt_addr + 36 + i * 4;
        if !addr_range_safe(entry_addr, 4) {
            continue;
        }
        let table_addr = unsafe { read_u32(entry_addr) };
        if signature_matches(table_addr, b"FACP") {
            return Some(table_addr);
        }
    }
    None
}

/// Validates and reads the fixed region every field above lives in
/// (72 bytes - the real, fixed ACPI 1.0 FADT layout up through
/// PM1b_CNT_BLK at offset 68..72, confirmed against the spec:
/// DSDT@40, SMI_CMD@48, ACPI_ENABLE@52, PM1a_CNT_BLK@64,
/// PM1b_CNT_BLK@68) upfront, the same "whole fixed region bounds-
/// checked before any field read, not field-by-field" discipline
/// `find_madt()`/`parse_madt()` above already established.
fn parse_fadt(fadt_addr: u32) -> Option<FadtFields> {
    if !addr_range_safe(fadt_addr, 72) {
        return None;
    }
    if !signature_matches(fadt_addr, b"FACP") {
        return None;
    }
    let length = unsafe { read_u32(fadt_addr + 4) };
    if length < 72 || !checksum_ok(fadt_addr, length) {
        return None;
    }

    Some(FadtFields {
        dsdt_addr: unsafe { read_u32(fadt_addr + 40) },
        smi_cmd: unsafe { read_u32(fadt_addr + 48) },
        acpi_enable: unsafe { read_u8(fadt_addr + 52) },
        pm1a_cnt_blk: unsafe { read_u32(fadt_addr + 64) },
        pm1b_cnt_blk: unsafe { read_u32(fadt_addr + 68) },
    })
}

/// Decodes the handful of AML bytes immediately following an already-
/// located `_S5_` name into `(SLP_TYPa, SLP_TYPb)`. `cursor` must
/// point exactly one byte past the `_S5_` name's own 4 ASCII bytes;
/// `end` bounds every read to the DSDT's own declared length (checked
/// by the caller, `find_s5_sleep_type()`, before this is ever called).
///
/// Expected encoding, per how every ACPI-compiler-generated DSDT
/// declares `Name (_S5, Package (0x04) { SLP_TYPa, SLP_TYPb, 0, 0 })`:
/// `PackageOp`(0x12) `PkgLength` `NumElements` `SLP_TYPa` `SLP_TYPb`
/// ... Two encodings intentionally handled, not one: `PkgLength`'s own
/// top two bits say how many *additional* bytes encode the actual
/// length value (0-3 more) - this function skips exactly that many
/// bytes without ever computing the length itself, since nothing here
/// needs to know it, only skip past it correctly. Each `SLP_TYP`
/// integer (always 0-7, a 3-bit hardware field) is compiler-encoded
/// either as a raw byte preceded by `BytePrefix`(0x0A), or - for the
/// specific values 0 and 1 - as `ZeroOp`(0x00) or `OneOp`(0x01) alone,
/// with no prefix at all. This function does not need to tell those
/// two cases apart: `ZeroOp`'s and `OneOp`'s own byte values (0x00,
/// 0x01) already equal the integers 0 and 1 they mean, so "skip a
/// 0x0A prefix if present, then use whatever byte is there" reads the
/// correct value either way - the same well-known technique
/// OSDev.org's own "Shutdown" reference page documents, adapted here
/// with real bounds checking added at every single byte, since that
/// reference implementation (correctly, for a page whose whole point
/// is the AML decoding, not memory safety) has none at all.
fn try_parse_s5_package(mut cursor: u32, end: u32) -> Option<(u16, u16)> {
    if cursor >= end || !addr_range_safe(cursor, 1) {
        return None;
    }
    if unsafe { read_u8(cursor) } != 0x12 {
        return None; // not immediately followed by PackageOp - not the
                      // shape this function knows how to decode
    }
    cursor += 1;

    if cursor >= end || !addr_range_safe(cursor, 1) {
        return None;
    }
    let pkglength_byte0 = unsafe { read_u8(cursor) };
    let extra_length_bytes = (pkglength_byte0 >> 6) & 0x3;
    cursor += 1 + extra_length_bytes as u32; // skip PkgLength entirely

    // NumElements (1 byte) - skipped, not validated; the two elements
    // this function actually reads are always the package's first two
    // regardless of how many more follow.
    if cursor >= end || !addr_range_safe(cursor, 1) {
        return None;
    }
    cursor += 1;

    let read_typ = |c: &mut u32| -> Option<u16> {
        if *c >= end || !addr_range_safe(*c, 1) {
            return None;
        }
        let mut b = unsafe { read_u8(*c) };
        if b == 0x0A {
            // BytePrefix - the real value is the next byte
            *c += 1;
            if *c >= end || !addr_range_safe(*c, 1) {
                return None;
            }
            b = unsafe { read_u8(*c) };
        }
        *c += 1;
        Some(b as u16)
    };

    let slp_typa = read_typ(&mut cursor)?;
    let slp_typb = read_typ(&mut cursor)?;
    Some((slp_typa, slp_typb))
}

/// Scans a DSDT's own bytes (bounds-checked against both its own
/// declared length and this kernel's identity-mapped range, exactly
/// like every other table walk in this module) for the literal 4-byte
/// ASCII name `_S5_` (the trailing underscore is part of the real
/// AML name - every ACPI NameSeg is padded to exactly 4 characters
/// with trailing `_`), then hands off to `try_parse_s5_package()`
/// immediately after each match. A DSDT can legitimately be tens of
/// kilobytes - unlike every fixed-size table this module reads
/// elsewhere, this is a real linear byte scan over real firmware-
/// supplied data, not a fixed handful of field reads, so it's
/// deliberately un-aligned (`_S5_` can start at any byte offset,
/// unlike the RSDP's own 16-byte-aligned scan) and bounded strictly by
/// the DSDT's own checksummed length.
fn find_s5_sleep_type(dsdt_addr: u32) -> Option<(u16, u16)> {
    if !addr_range_safe(dsdt_addr, 36) {
        return None;
    }
    if !signature_matches(dsdt_addr, b"DSDT") {
        return None;
    }
    let length = unsafe { read_u32(dsdt_addr + 4) };
    if length < 36 || !addr_range_safe(dsdt_addr, length)
        || !checksum_ok(dsdt_addr, length)
    {
        return None;
    }

    let end = dsdt_addr + length;
    const PATTERN: &[u8; 4] = b"_S5_";
    let mut addr = dsdt_addr + 36; // search the table body, after its
                                    // own 36-byte SDT header
    while addr + 4 <= end {
        if !addr_range_safe(addr, 4) {
            break;
        }
        let mut matched = true;
        for i in 0..4u32 {
            if unsafe { read_u8(addr + i) } != PATTERN[i as usize] {
                matched = false;
                break;
            }
        }
        if matched {
            if let Some(result) = try_parse_s5_package(addr + 4, end) {
                return Some(result);
            }
            // A byte-for-byte match on "_S5_" that isn't followed by a
            // decodable package is treated as a false positive (AML
            // string/name data can coincidentally contain this exact
            // 4-byte sequence) - keep scanning rather than give up.
        }
        addr += 1;
    }
    None
}

/// `pub(crate)`, not private, as of Phase 56 - kernel/rust/apic.rs
/// reuses these three directly (masking the legacy 8259 PIC once
/// IO-APIC routing takes over needs the exact same port I/O primitive
/// this module already established and proved correct in Phase 55;
/// duplicating them there would be the same three `asm!` blocks
/// copy-pasted for no reason).
#[inline(always)]
pub(crate) unsafe fn port_outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value,
                      options(nomem, nostack, preserves_flags));
}

#[inline(always)]
pub(crate) unsafe fn port_outw(port: u16, value: u16) {
    core::arch::asm!("out dx, ax", in("dx") port, in("ax") value,
                      options(nomem, nostack, preserves_flags));
}

#[inline(always)]
pub(crate) unsafe fn port_inw(port: u16) -> u16 {
    let value: u16;
    core::arch::asm!("in ax, dx", in("dx") port, out("ax") value,
                      options(nomem, nostack, preserves_flags));
    value
}

// ============================================================
// Phase 56: MADT I/O APIC (type 1) and Interrupt Source Override
// (type 2) entries - the two additional pieces of the same table
// Phase 44 already parses for CPU-topology (type 0) entries, needed
// before kernel/rust/apic.rs can program a real IO-APIC in place of
// the legacy 8259 PIC.
// ============================================================

/// Bounded the same way `AcpiCpuDiscovery::apic_ids` is (see its own
/// comment) - real machines overwhelmingly have exactly one IO-APIC;
/// this cap exists so a pathological table can't grow this struct's
/// fixed-size array without bound, not because more are expected.
const MAX_IO_APICS: usize = 4;

/// Bounded the same way, for Interrupt Source Override entries - a
/// real MADT rarely has more than one or two (the IRQ0 override is
/// the one this kernel actually depends on - see `smp_discover()`'s
/// own comment), but nothing stops a table from listing one for every
/// ISA IRQ line.
const MAX_OVERRIDES: usize = 16;

#[derive(Clone, Copy)]
pub(crate) struct IoApicInfo {
    pub(crate) id: u8,
    pub(crate) phys_addr: u32,
    pub(crate) gsi_base: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct IrqOverride {
    pub(crate) isa_irq: u8,
    pub(crate) gsi: u32,
}

/// Everything kernel/rust/apic.rs needs to actually bring up SMP and
/// replace the PIC, in one place - a strict superset of what
/// `AcpiCpuDiscovery` (Phase 44, kept as-is and still used by
/// `rust_acpi_discover_cpus()`'s own existing FFI callers) provides.
/// Built by a *separate* parse of the same MADT, deliberately not by
/// extending `parse_madt()` itself in place - Phase 44's own parsing
/// logic is already shipped, tested (including against three
/// different real `-smp N` configurations, not just the synthetic
/// self-test), and depended on by main.c's existing informational log
/// line; re-deriving the CPU list here from scratch, in new code, is
/// a small amount of duplication in exchange for not touching a
/// working, already-proven code path at all.
pub(crate) struct SmpDiscovery {
    pub(crate) found_acpi: bool,
    pub(crate) found_madt: bool,
    pub(crate) cpu_count: u32,
    pub(crate) apic_ids: [u8; MAX_CPUS],
    pub(crate) local_apic_phys: u32,
    pub(crate) io_apic_count: u32,
    pub(crate) io_apics: [IoApicInfo; MAX_IO_APICS],
    pub(crate) override_count: u32,
    pub(crate) overrides: [IrqOverride; MAX_OVERRIDES],
}

impl SmpDiscovery {
    const fn empty() -> Self {
        SmpDiscovery {
            found_acpi: false,
            found_madt: false,
            cpu_count: 0,
            apic_ids: [0; MAX_CPUS],
            local_apic_phys: 0,
            io_apic_count: 0,
            io_apics: [IoApicInfo { id: 0, phys_addr: 0, gsi_base: 0 }; MAX_IO_APICS],
            override_count: 0,
            overrides: [IrqOverride { isa_irq: 0, gsi: 0 }; MAX_OVERRIDES],
        }
    }

    /// The GSI a legacy ISA IRQ line actually shows up on for this
    /// specific machine - identity (`isa_irq as u32`) unless an
    /// Interrupt Source Override entry says otherwise. This matters
    /// for exactly one line in practice on PC/AT-compatible chipsets
    /// (including QEMU's own default `q35`/`i440fx` machine types):
    /// ISA IRQ0 (the PIT) is very commonly wired to GSI 2, not GSI 0,
    /// because GSI 0 is reserved for a different purpose in the
    /// IO-APIC's own fixed wiring - a real, well-known, independently
    /// documented (OSDev.org's own "IOAPIC" page covers exactly this)
    /// gotcha, not a rare edge case. Getting this wrong would silently
    /// mean the timer IRQ this kernel's entire scheduler tick depends
    /// on (kernel/drivers/timer's own IRQ0 registration) is redirected
    /// to a GSI nothing is actually listening on, the moment IO-APIC
    /// routing replaces the PIC - the single most important reason
    /// this module parses Interrupt Source Override entries at all,
    /// rather than assuming GSI == IRQ for every ISA line the way a
    /// first, simpler draft of this function did.
    pub(crate) fn gsi_for_isa_irq(&self, isa_irq: u8) -> u32 {
        for i in 0..(self.override_count as usize) {
            if self.overrides[i].isa_irq == isa_irq {
                return self.overrides[i].gsi;
            }
        }
        isa_irq as u32
    }
}

fn parse_madt_smp(madt_addr: u32, out: &mut SmpDiscovery) {
    // Same fixed-size-header-first validation as parse_madt() above -
    // see that function's own comment for why the order matters.
    if !addr_range_safe(madt_addr, 44) {
        return;
    }
    if !signature_matches(madt_addr, b"APIC") {
        return;
    }
    let length = unsafe { read_u32(madt_addr + 4) };
    if length < 44 || !checksum_ok(madt_addr, length) {
        return;
    }
    out.found_madt = true;
    out.local_apic_phys = unsafe { read_u32(madt_addr + 36) };

    let end = madt_addr + length;
    let mut cursor = madt_addr + 44;

    while cursor + 2 <= end {
        if !addr_range_safe(cursor, 2) {
            break;
        }
        let entry_type = unsafe { read_u8(cursor) };
        let entry_len = unsafe { read_u8(cursor + 1) };
        if entry_len == 0 {
            break;
        }
        if entry_type == 0 && entry_len >= 8 && cursor + 8 <= end
            && addr_range_safe(cursor, 8)
        {
            // Processor Local APIC (see parse_madt()'s own comment
            // for the field layout).
            let apic_id = unsafe { read_u8(cursor + 3) };
            let flags = unsafe { read_u32(cursor + 4) };
            let enabled = (flags & 1) != 0;
            if enabled && (out.cpu_count as usize) < MAX_CPUS {
                out.apic_ids[out.cpu_count as usize] = apic_id;
                out.cpu_count += 1;
            }
        } else if entry_type == 1 && entry_len >= 12 && cursor + 12 <= end
            && addr_range_safe(cursor, 12)
        {
            // I/O APIC entry (ACPI spec table 5-27): type(1) length(1)
            // io_apic_id(1) reserved(1) io_apic_address(4)
            // global_system_interrupt_base(4).
            if (out.io_apic_count as usize) < MAX_IO_APICS {
                let id = unsafe { read_u8(cursor + 2) };
                let phys_addr = unsafe { read_u32(cursor + 4) };
                let gsi_base = unsafe { read_u32(cursor + 8) };
                out.io_apics[out.io_apic_count as usize] =
                    IoApicInfo { id, phys_addr, gsi_base };
                out.io_apic_count += 1;
            }
        } else if entry_type == 2 && entry_len >= 10 && cursor + 10 <= end
            && addr_range_safe(cursor, 10)
        {
            // Interrupt Source Override (ACPI spec table 5-29):
            // type(1) length(1) bus(1) source(1)
            // global_system_interrupt(4) flags(2). `bus` is always 0
            // (ISA) in every real table this kernel has ever seen -
            // not checked, since a nonzero value here would still be
            // a real override this kernel should honor, not ignore.
            if (out.override_count as usize) < MAX_OVERRIDES {
                let isa_irq = unsafe { read_u8(cursor + 3) };
                let gsi = unsafe { read_u32(cursor + 4) };
                out.overrides[out.override_count as usize] =
                    IrqOverride { isa_irq, gsi };
                out.override_count += 1;
            }
        }
        cursor += entry_len as u32;
    }
}

/// The Phase 56 analogue of `discover()` above - same RSDP -> RSDT ->
/// MADT walk, richer result. `pub(crate)`: called directly from
/// kernel/rust/apic.rs, entirely within this crate - no FFI boundary,
/// no repr(C) struct, no bool-sizing hazard, since both sides are
/// ordinary Rust talking to ordinary Rust.
pub(crate) fn discover_smp() -> SmpDiscovery {
    let mut result = SmpDiscovery::empty();

    let rsdp_addr = match find_rsdp() {
        Some(addr) => addr,
        None => return result,
    };
    result.found_acpi = true;

    let rsdt_addr = unsafe { read_u32(rsdp_addr + 16) };
    if let Some(madt_addr) = find_madt(rsdt_addr) {
        parse_madt_smp(madt_addr, &mut result);
    }

    result
}

/// Runs the real discovery path (RSDP -> RSDT -> FADT -> DSDT -> _S5)
/// against whatever ACPI tables this machine actually provides -
/// shared by both `rust_acpi_shutdown_info()` (logs what was found,
/// changes nothing) and `rust_acpi_shutdown()` (acts on it). Reuses
/// `find_rsdp()` above rather than caching Phase 44's own result:
/// this runs at most a small handful of times per boot (an optional
/// boot-time log, and at most one real shutdown attempt), so re-
/// scanning is not worth adding shared mutable state to avoid.
fn discover_shutdown_info() -> AcpiShutdownInfo {
    let mut info = AcpiShutdownInfo::empty();

    let rsdp_addr = match find_rsdp() {
        Some(addr) => addr,
        None => return info,
    };
    info.found_acpi = 1;

    let rsdt_addr = unsafe { read_u32(rsdp_addr + 16) };
    let fadt_addr = match find_fadt(rsdt_addr) {
        Some(addr) => addr,
        None => return info,
    };
    let fadt = match parse_fadt(fadt_addr) {
        Some(f) => f,
        None => return info,
    };
    info.found_fadt = 1;
    info.pm1a_cnt_blk = fadt.pm1a_cnt_blk;
    info.pm1b_cnt_blk = fadt.pm1b_cnt_blk;
    info.smi_cmd = fadt.smi_cmd;
    info.acpi_enable = fadt.acpi_enable;

    if fadt.pm1a_cnt_blk != 0 && fadt.pm1a_cnt_blk <= 0xFFFF {
        let cnt = unsafe { port_inw(fadt.pm1a_cnt_blk as u16) };
        info.sci_en_already_set = if (cnt & PM1_SCI_EN) != 0 { 1 } else { 0 };
    }

    if let Some((slp_typa, slp_typb)) = find_s5_sleep_type(fadt.dsdt_addr) {
        info.found_s5 = 1;
        info.slp_typa = slp_typa;
        info.slp_typb = slp_typb;
    }

    info
}

/// Non-destructive discovery, safe to run and log at every boot (see
/// kernel/init/main.c's own call site): finds and reports exactly
/// what `rust_acpi_shutdown()` below would use, without ever touching
/// an I/O port that changes machine state. Returns -1 (no ACPI at
/// all), -2 (ACPI found but no usable FADT), -3 (FADT found but no
/// `_S5` package could be located in its DSDT), or 0 (everything a
/// real shutdown needs was found) - `out` is filled in regardless of
/// the return value, with whatever was actually discovered before the
/// point of failure.
#[no_mangle]
pub extern "C" fn rust_acpi_shutdown_info(out: *mut AcpiShutdownInfo) -> i32 {
    let info = discover_shutdown_info();
    unsafe {
        *out = info;
    }
    if info.found_acpi == 0 {
        -1
    } else if info.found_fadt == 0 || info.pm1a_cnt_blk == 0 {
        -2
    } else if info.found_s5 == 0 {
        -3
    } else {
        0
    }
}

/// The real thing: discovers this machine's own FADT/PM1_CNT/_S5
/// values (exactly as `rust_acpi_shutdown_info()` above does - the two
/// deliberately share `discover_shutdown_info()` rather than one
/// trusting the other's earlier result, so this always acts on a
/// fresh, direct read of this machine's own tables), enables ACPI if
/// it isn't already, then writes the real S5 sleep-state request to
/// PM1a_CNT_BLK (and PM1b_CNT_BLK too, if this machine has a second
/// one - most, including QEMU, don't).
///
/// On real, working ACPI hardware this function does not return at
/// all - the write itself powers the machine off. Every negative
/// return value below is therefore a real, honestly-distinguished
/// failure to get there, not a success code that was forgotten: -1 no
/// ACPI present at all, -2 no usable FADT/PM1a_CNT_BLK, -3 no `_S5`
/// package found in the DSDT, -4 ACPI needs enabling first but this
/// FADT gives no SMI_CMD/ACPI_ENABLE to do it with, -5 the enable
/// sequence was issued but SCI_EN never became set within this
/// function's own bounded wait, -6 SLP_EN was written to PM1_CNT and
/// this function is still running afterward - the tables claimed S5
/// support but the real hardware didn't act on it.
#[no_mangle]
pub extern "C" fn rust_acpi_shutdown() -> i32 {
    let info = discover_shutdown_info();

    if info.found_acpi == 0 {
        return -1;
    }
    if info.found_fadt == 0 || info.pm1a_cnt_blk == 0 || info.pm1a_cnt_blk > 0xFFFF {
        return -2;
    }
    if info.found_s5 == 0 {
        return -3;
    }

    if info.sci_en_already_set == 0 {
        if info.smi_cmd == 0 || info.smi_cmd > 0xFFFF || info.acpi_enable == 0 {
            return -4;
        }
        unsafe {
            port_outb(info.smi_cmd as u16, info.acpi_enable);
        }
        let mut enabled = false;
        let mut i = 0u32;
        while i < BUSY_WAIT_ITERATIONS {
            let cnt = unsafe { port_inw(info.pm1a_cnt_blk as u16) };
            if (cnt & PM1_SCI_EN) != 0 {
                enabled = true;
                break;
            }
            i += 1;
        }
        if !enabled {
            return -5;
        }
    }

    let value_a = ((info.slp_typa & 0x7) << 10) | PM1_SLP_EN;
    unsafe {
        port_outw(info.pm1a_cnt_blk as u16, value_a);
    }
    if info.pm1b_cnt_blk != 0 && info.pm1b_cnt_blk <= 0xFFFF {
        let value_b = ((info.slp_typb & 0x7) << 10) | PM1_SLP_EN;
        unsafe {
            port_outw(info.pm1b_cnt_blk as u16, value_b);
        }
    }

    // Still executing means the write didn't actually power the
    // machine off - spin briefly (bounded, same non-timer-based
    // reasoning as the enable wait above) before honestly reporting
    // failure rather than either hanging forever or returning
    // immediately on hardware that just needed a moment.
    let mut i = 0u32;
    while i < BUSY_WAIT_ITERATIONS {
        core::hint::spin_loop();
        i += 1;
    }
    -6
}

/// Ring-0 self-test - the same "small, fully synthetic, hand-
/// constructed table this test controls byte-for-byte" methodology
/// `rust_acpi_selftest()` above already established for MADT parsing,
/// applied here to FADT parsing and, new to this phase, the `_S5` AML
/// decode: no real machine's DSDT is a "known correct answer" to
/// check `find_s5_sleep_type()` against in isolation, so this builds
/// one itself. Deliberately does NOT call `rust_acpi_shutdown()` at
/// all - that function's entire purpose is to change real machine
/// state (power it off), which a boot-time self-test must never risk
/// doing by accident.
#[no_mangle]
pub extern "C" fn rust_acpi_fadt_selftest() -> i32 {
    let mut code = 0;

    // Synthetic DSDT: a 36-byte SDT header, immediately followed (no
    // preceding AML filler - harmless, since find_s5_sleep_type()
    // scans byte-by-byte regardless of what precedes a match) by
    // `_S5_`, PackageOp, a 1-byte PkgLength (its actual value is never
    // read by this module - only its top two bits, both 0 here,
    // meaning "no additional length bytes"), NumElements(4), and the
    // two SLP_TYP values (5 and 7 - arbitrary, chosen specifically to
    // be distinguishable from each other and from 0/1, so this test
    // cannot pass by accident) each BytePrefix-encoded, plus two more
    // ZeroOp elements completing the realistic 4-element package
    // (unread by the parser, present only for realism).
    let mut dsdt = [0u8; 49];
    dsdt[0..4].copy_from_slice(b"DSDT");
    let dsdt_len = dsdt.len() as u32;
    dsdt[4..8].copy_from_slice(&dsdt_len.to_le_bytes());
    dsdt[36..40].copy_from_slice(b"_S5_");
    dsdt[40] = 0x12; // PackageOp
    dsdt[41] = 0x08; // PkgLength, 1-byte form (top 2 bits clear)
    dsdt[42] = 0x04; // NumElements
    dsdt[43] = 0x0A; // BytePrefix
    dsdt[44] = 5; // SLP_TYPa
    dsdt[45] = 0x0A; // BytePrefix
    dsdt[46] = 7; // SLP_TYPb
    dsdt[47] = 0x00; // ZeroOp (3rd element, never read by this parser)
    dsdt[48] = 0x00; // ZeroOp (4th element, never read by this parser)
    let dsdt_addr = dsdt.as_ptr() as u32;
    let dsdt_sum: u8 = dsdt
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != 9) // checksum byte itself, SDT offset 9
        .fold(0u8, |acc, (_, &b)| acc.wrapping_add(b));
    dsdt[9] = (0u8).wrapping_sub(dsdt_sum);

    match find_s5_sleep_type(dsdt_addr) {
        Some((a, b)) if a == 5 && b == 7 => {}
        _ => code |= 1,
    }

    // Synthetic FADT: only the fields this module actually reads are
    // load-bearing (see FadtFields's own doc comment) - every other
    // real FADT field is left zeroed. SMI_CMD/ACPI_ENABLE use their
    // real, standard values (0xB2/0xA0 - the same ones QEMU/SeaBIOS
    // itself uses), and PM1a_CNT_BLK uses QEMU's own real, standard
    // value (0x604) - realistic, not load-bearing for what this test
    // actually checks (this test never issues a real port write).
    let mut fadt = [0u8; 72];
    fadt[0..4].copy_from_slice(b"FACP");
    let fadt_len = fadt.len() as u32;
    fadt[4..8].copy_from_slice(&fadt_len.to_le_bytes());
    fadt[40..44].copy_from_slice(&dsdt_addr.to_le_bytes());
    fadt[48..52].copy_from_slice(&0xB2u32.to_le_bytes()); // SMI_CMD
    fadt[52] = 0xA0; // ACPI_ENABLE
    fadt[64..68].copy_from_slice(&0x604u32.to_le_bytes()); // PM1a_CNT_BLK
    fadt[68..72].copy_from_slice(&0u32.to_le_bytes()); // PM1b_CNT_BLK: none
    let fadt_addr = fadt.as_ptr() as u32;
    let fadt_sum: u8 = fadt
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != 9)
        .fold(0u8, |acc, (_, &b)| acc.wrapping_add(b));
    fadt[9] = (0u8).wrapping_sub(fadt_sum);

    match parse_fadt(fadt_addr) {
        Some(f)
            if f.dsdt_addr == dsdt_addr
                && f.smi_cmd == 0xB2
                && f.acpi_enable == 0xA0
                && f.pm1a_cnt_blk == 0x604
                && f.pm1b_cnt_blk == 0 => {}
        _ => code |= 2,
    }

    // The negative case, proven directly rather than assumed: a
    // corrupted FADT (bad checksum) must be rejected outright, not
    // silently trusted - the same "prove the rejection case too"
    // discipline kernel/rust/journal.rs's own two-part self-test
    // already applies to recovery.
    let mut bad_fadt = fadt;
    bad_fadt[9] = bad_fadt[9].wrapping_add(1);
    if parse_fadt(bad_fadt.as_ptr() as u32).is_some() {
        code |= 4;
    }

    code
}
