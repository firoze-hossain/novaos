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
//! than left as a mysterious "sometimes doesn't work": this module
//! can only safely read memory within this kernel's own identity-
//! mapped range (paging.c's own confirmed 0-64MB - see
//! IDENTITY_MAPPED_LIMIT below), and on this project's own default
//! `-m 512M` test config, QEMU places the actual RSDT/MADT tables
//! well above that boundary - discovery correctly, gracefully reports
//! "not found" in that case (proven safe: an earlier version of this
//! module read those tables' signature/length *before* checking
//! bounds at all, and crashed with a real page fault the first time
//! it ran against real hardware instead of the self-test's own
//! synthetic, stack-allocated table). Confirmed this is a memory-size
//! boundary issue, not a parsing bug, by testing with `-m 32M`
//! (comfortably under 64MB) instead: real discovery then succeeds,
//! and the reported CPU count was independently verified against
//! three different `-smp N` values (default/1, 2, and 4), each
//! matching exactly - see PROGRESS.md's Phase 44 entry for the full
//! account. Extending this kernel's identity map to cover more of
//! physical memory would close this gap, but is real, separate,
//! larger-blast-radius work (touching paging.c, not this module) -
//! deliberately not attempted in this same, otherwise low-risk phase.

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

/// Reads a `u8` from a raw physical address - safe on this kernel
/// specifically because paging.c identity-maps the entire 0-64MB
/// range (confirmed directly, not assumed, before writing this
/// module: `kernel_log("[ OK ] Paging enabled (identity-mapped
/// 0-64MB)")`), and every address this module ever reads (the EBDA
/// pointer at physical 0x40E, the 0xE0000-0xFFFFF BIOS ROM range, and
/// whatever RSDT/MADT addresses those tables themselves report,
/// bounds-checked against that same range below) falls well within it.
///
/// # Safety
/// `addr` must be a physical address known to be mapped and safe to
/// read as plain memory - every call site below either uses a fixed,
/// known-safe address or one bounds-checked against the identity-
/// mapped range first.
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

/// This kernel's own identity-mapped range (see paging.c's own log
/// message, confirmed directly before relying on it: "Paging enabled
/// (identity-mapped 0-64MB)") - the only physical memory this module
/// can safely read at all. Real ACPI tables are not guaranteed to sit
/// within it - a QEMU/real-firmware RSDT or MADT can legitimately be
/// placed anywhere in physical memory, and on this project's own
/// `-m 512M` test config, they routinely are, well above 64MB. Every
/// read in this module must be bounds-checked against this range
/// *before* the read happens, not after - the bug this comment exists
/// to prevent a regression of: an earlier version of this module
/// checked bounds only inside checksum_ok(), called *after* already
/// reading a table's signature and length first, which crashed with a
/// real page fault the first time this ran against QEMU's actual
/// ACPI tables instead of the self-test's own stack-allocated
/// synthetic one (stack memory is always within the identity-mapped
/// range, which is exactly why the self-test alone didn't catch this).
const IDENTITY_MAPPED_LIMIT: u32 = 0x4000000; // 64MB

/// Must be checked before any read at `addr` - true iff every byte in
/// `[addr, addr+len)` is safely within this kernel's identity-mapped
/// range. Uses `checked_add` deliberately, not plain `+`, so a
/// pathological `len` large enough to overflow `u32` arithmetic is
/// rejected rather than wrapping into a false "safe" result.
fn addr_range_safe(addr: u32, len: u32) -> bool {
    len != 0
        && addr
            .checked_add(len)
            .map(|end| end <= IDENTITY_MAPPED_LIMIT)
            .unwrap_or(false)
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
                                           out_found_acpi: *mut bool)
                                           -> bool {
    let result = discover();
    unsafe {
        *out_count = result.cpu_count;
        *out_local_apic_phys = result.local_apic_phys;
        *out_found_acpi = result.found_acpi;
    }
    result.found_madt
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
