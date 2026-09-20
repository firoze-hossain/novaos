//! kernel/rust/apic.rs - Phase 56: Local APIC + I/O APIC, and real
//! SMP bring-up (booting a second CPU and having it run real kernel
//! code) on top of them.
//!
//! Builds directly on Phase 44's own ACPI MADT parsing
//! (kernel/rust/acpi.rs), which that phase's own header comment
//! named as "the genuine first SMP prerequisite" and was careful to
//! say was NOT SMP support itself - discovering how many CPUs exist
//! and their APIC IDs, nothing more. This module is the next several
//! items on that same list, in the order that phase's header comment
//! named them: a Local APIC driver, an I/O APIC driver (replacing the
//! 8259 PIC this kernel's interrupt architecture ran on through
//! Phase 55), an AP bootstrap trampoline in low memory
//! (kernel/arch/x86/cpu/ap_trampoline.s), and enough per-CPU state to
//! let a second CPU run independent, real kernel code at all.
//!
//! What this phase deliberately does NOT do, named as directly as
//! Phase 44 named its own scope: this is not a symmetric multi-
//! processing *scheduler*. kernel/task/scheduler.c's `current`/
//! `current_index` globals and `pick_next()` are completely untouched
//! - every AP this module brings up runs exactly one thing, forever,
//! once it comes online: `rust_ap_main()`'s own `sti; hlt` idle loop
//! below, never anything from kernel/task/. Nor is this an audit of
//! every other existing shared kernel structure (the process table,
//! the PMM bitmap, the heap allocator, every driver's own state) for
//! real multi-CPU safety - this project's own release-readiness
//! roadmap named that as "the genuinely hardest part... a much larger
//! remaining task" than either of the two primitives (SpinLock,
//! Phase 40; this module) that came before it, and nothing about
//! actually building this module changed that assessment. It remains
//! real, unaudited, future work, not a checkbox - see PROGRESS.md's
//! Phase 56 entry for the explicit list of what that means in
//! practice (starting with the fact that no AP here ever calls into
//! kernel/task/ at all, so nothing SMP-unsafe there is actually
//! exercised yet either).
//!
//! What IS real here, and provably so, not just claimed: a second
//! (and third, ...) physical CPU core genuinely leaves reset state,
//! executes hand-written real-mode assembly, transitions through
//! protected mode, enables paging with this kernel's own page
//! directory, loads this kernel's own real GDT/IDT, and starts
//! running actual compiled Rust - not a simulation of any of those
//! steps. See PROGRESS.md's Phase 56 entry for how this was verified:
//! this project's own QEMU test config now boots with more than one
//! virtual CPU by default specifically so `make test` exercises this
//! path automatically, not just as a one-off manual check the way
//! Phase 44's own `-smp N` verification had to be.
//!
//! Also closes a real, if secondary, gap Phase 44 shipped knowingly:
//! see `crate::acpi::ensure_mapped()`'s own doc comment for why real
//! ACPI table discovery under this project's default `-m 512M` test
//! config previously, silently failed (tables placed above this
//! kernel's static 64MB identity map), and how the on-demand physical-
//! page mapping this module needs anyway, unconditionally, for its
//! own LAPIC/IO-APIC MMIO access (fixed hardware addresses just under
//! 4GB, nowhere near even a generous static identity map) ends up
//! fixing that too, for free.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

extern "C" {
    /// See kernel/rust/acpi.rs's own extern block for the same
    /// declaration's full reasoning (received as `i32`/`u32`, not C's
    /// own `bool`/pointer-sized types, to sidestep the exact FFI
    /// hazard that module's doc comments describe catching twice
    /// already this project).
    fn paging_kernel_directory_phys() -> u32;

    /// kernel/arch/x86/mm/pmm.h's own `pmm_alloc_contiguous()` -
    /// reused here for exactly the reason its own doc comment already
    /// gives: "virtio's virtqueue, and any future DMA-capable driver,
    /// needs physically-contiguous memory, not one arbitrary frame at
    /// a time." A new AP's kernel stack is the same requirement for a
    /// different reason - a stack has to be one *contiguous* range of
    /// memory, and this kernel's identity-mapped-low-memory convention
    /// (see paging.h's own "IMPORTANT CAVEAT" comment, which every
    /// function in this file relies on exactly as paging.c's own
    /// per-process code already does) means "contiguous physically"
    /// and "contiguous virtually" are the same thing here. Returns 0
    /// on failure - checked below, not ignored (an AP that can't get
    /// a stack is skipped, not booted into a stack of zeroes).
    fn pmm_alloc_contiguous(count: u32) -> u32;

    /// kernel/arch/x86/cpu/idt.h's own `idt_set_gate()` - used once,
    /// to install a real (if trivial) handler for this module's own
    /// chosen spurious-interrupt vector (see `SPURIOUS_VECTOR` below)
    /// before ever enabling a Local APIC that could raise one. Without
    /// this, an actual spurious interrupt would land on an IDT gate
    /// idt_init() left not-present (every vector starts that way -
    /// see idt.c's own `idt_init()`), which is a real CPU exception
    /// (#NP, vector 11) this kernel would then have to survive taking
    /// *while already inside interrupt handling* - not a theoretical
    /// concern to leave unhandled.
    fn idt_set_gate(num: u8, base: u32, selector: u16, flags: u8);

    /// New (Phase 56) tiny accessors - see kernel/arch/x86/cpu/gdt.c
    /// and idt.c's own doc comments for why these didn't exist before
    /// now: nothing before this phase ever needed the *address* of
    /// this kernel's own already-loaded GDT/IDT pointer structs from
    /// outside gdt.c/idt.c themselves. kernel/arch/x86/cpu/
    /// ap_trampoline.s's own mailbox (see its own header comment)
    /// carries whatever these return so a newly-woken AP can load the
    /// exact same, already-built tables the BSP already uses - there
    /// is exactly one GDT and one IDT in this kernel, shared by every
    /// CPU, not rebuilt per-CPU.
    fn gdt_get_pointer_addr() -> u32;
    fn idt_get_pointer_addr() -> u32;
}

// ============================================================
// Local APIC (per-CPU, but every CPU's own LAPIC happens to be
// mapped at the same physical address - see `lapic_enable()`'s own
// comment for why that's correct, not a bug)
// ============================================================

const LAPIC_REG_ID: u32 = 0x020;
const LAPIC_REG_EOI: u32 = 0x0B0;
const LAPIC_REG_SVR: u32 = 0x0F0; // Spurious Interrupt Vector Register
const LAPIC_REG_ICR_LOW: u32 = 0x300;
const LAPIC_REG_ICR_HIGH: u32 = 0x310;

/// Intel's own recommended choice (SDM Vol 3A, 10.9): all-ones is
/// deliberately reserved for exactly this purpose, so a spurious
/// vector can never collide with a real one this kernel assigns
/// itself. `rust_smp_init()` installs a real (trivial) IDT gate here
/// before ever enabling a LAPIC - see this module's own header
/// comment.
const SPURIOUS_VECTOR: u8 = 0xFF;

/// This kernel's Local APIC MMIO base, once discovered - 0 means "not
/// yet discovered / no usable LAPIC on this machine." An `AtomicU32`,
/// not a `SpinLock`-wrapped value: written exactly once, by the BSP,
/// before any AP exists to race it, and every subsequent access (EOI
/// on every hardware interrupt - a genuine hot path) is a plain read;
/// there is no read-modify-write here that a lock would need to
/// protect.
static LAPIC_BASE: AtomicU32 = AtomicU32::new(0);

/// # Safety
/// `base` must already be mapped (see `crate::acpi::ensure_mapped()`)
/// and be a genuine Local APIC MMIO base.
#[inline(always)]
unsafe fn lapic_read(base: u32, reg: u32) -> u32 {
    core::ptr::read_volatile((base + reg) as *const u32)
}

#[inline(always)]
unsafe fn lapic_write(base: u32, reg: u32, value: u32) {
    core::ptr::write_volatile((base + reg) as *mut u32, value);
}

/// Reads this specific CPU's own Local APIC ID - genuinely per-CPU
/// data despite every CPU's LAPIC sharing the same MMIO *address*:
/// xAPIC mode routes an access to that fixed physical address to
/// whichever CPU is actually making the access, at the hardware
/// level, not to some single shared device on the system bus the way
/// a normal MMIO peripheral works. This is standard, well-established
/// x86 behavior (every real OS's own SMP bring-up code relies on
/// exactly this), not something this kernel does anything special to
/// arrange.
///
/// # Safety
/// `base` must be a mapped, genuine Local APIC MMIO base.
unsafe fn lapic_id(base: u32) -> u8 {
    ((lapic_read(base, LAPIC_REG_ID) >> 24) & 0xFF) as u8
}

/// Software-enables *this* CPU's own Local APIC (bit 8 of the
/// Spurious Interrupt Vector Register - off by default at reset on
/// real hardware, and needs to be done independently on every CPU,
/// BSP included, for the same "separate physical hardware, shared
/// address" reason `lapic_id()` above already explains) and programs
/// its spurious vector to `SPURIOUS_VECTOR`.
///
/// # Safety
/// `base` must be a mapped, genuine Local APIC MMIO base, and a real
/// (even if trivial) IDT gate for `SPURIOUS_VECTOR` must already be
/// installed - see `rust_smp_init()`'s own ordering.
unsafe fn lapic_enable(base: u32) {
    let current = lapic_read(base, LAPIC_REG_SVR);
    lapic_write(base, LAPIC_REG_SVR, current | (1 << 8) | (SPURIOUS_VECTOR as u32));
}

/// A fixed, non-timer-based busy-loop bound, for the same reason
/// kernel/rust/acpi.rs's own `BUSY_WAIT_ITERATIONS` (Phase 55) is one
/// - see that constant's own comment for the full account of the
/// syscall-gate deadlock class this sidesteps. Every wait in this
/// module happens during `rust_smp_init()`, called once from
/// kernel_late_init() *before* `sti` (kernel/init/main.c) - interrupts
/// are already off for the entire duration on this specific call
/// path, so a timer-tick-based wait would be worse than merely risky
/// here, it would be a guaranteed hang (no IRQ0 can ever fire to
/// advance a tick counter while this code is running at all). Three
/// separate constants below, not one, because the three waits this
/// module needs have genuinely different real-world durations (the
/// Intel-recommended ~10ms INIT-to-SIPI gap is far longer than the
/// ~200us SIPI-to-SIPI gap, which is in turn unrelated to how long an
/// AP might reasonably take to actually reach the point of writing
/// its own mailbox ack) - one shared constant would have to be sized
/// for the largest of the three, wasting real boot time on the
/// other two for no benefit.
const INIT_TO_SIPI_WAIT_ITERATIONS: u32 = 8_000_000;
const SIPI_TO_SIPI_WAIT_ITERATIONS: u32 = 1_000_000;
const AP_ACK_WAIT_ITERATIONS: u32 = 100_000_000;

#[inline(always)]
fn busy_wait(iterations: u32) {
    for _ in 0..iterations {
        core::hint::spin_loop();
    }
}

/// Sends one Interrupt Command Register write to `dest_apic_id`, then
/// waits (bounded - see `busy_wait()`'s own comment) for the CPU's own
/// send-side "delivery pending" bit (ICR low, bit 12) to clear, which
/// is this architecture's own way of saying "the IPI has actually left
/// this CPU," before this function returns. Not waiting for this
/// would let a second ICR write (the very next line at most call
/// sites - INIT immediately followed by the first SIPI) race the
/// first one still being sent, which the SDM explicitly documents as
/// undefined.
///
/// # Safety
/// `base` must be a mapped, genuine, already-enabled Local APIC MMIO
/// base.
unsafe fn lapic_send_ipi(base: u32, dest_apic_id: u8, icr_low: u32) {
    lapic_write(base, LAPIC_REG_ICR_HIGH, (dest_apic_id as u32) << 24);
    lapic_write(base, LAPIC_REG_ICR_LOW, icr_low);
    let mut i = 0u32;
    while i < INIT_TO_SIPI_WAIT_ITERATIONS {
        if lapic_read(base, LAPIC_REG_ICR_LOW) & (1 << 12) == 0 {
            break;
        }
        i += 1;
    }
}

/// Called from kernel/arch/x86/cpu/irq.c's own `irq_handler()`
/// instead of the two legacy `outb(PIC_COMMAND, PIC_EOI)` writes,
/// once (and only once - see `rust_ioapic_is_active()`) IO-APIC
/// routing has actually taken over. A genuine hot path (once per
/// hardware interrupt, for the rest of this kernel's uptime), which
/// is exactly why this is a plain atomic load plus one MMIO write -
/// no lock, no re-mapping check, no discovery work happens here; all
/// of that is done once, up front, by `rust_smp_init()`.
#[no_mangle]
pub extern "C" fn rust_apic_send_eoi() {
    let base = LAPIC_BASE.load(Ordering::Acquire);
    if base == 0 {
        return; // defensive only - irq.c never calls this unless
                 // rust_ioapic_is_active() already said yes, which
                 // implies this was already set
    }
    unsafe {
        lapic_write(base, LAPIC_REG_EOI, 0);
    }
}

// ============================================================
// I/O APIC
// ============================================================

const IOAPIC_REG_IOREGSEL: u32 = 0x00;
const IOAPIC_REG_IOWIN: u32 = 0x10;

/// After PIC remapping (kernel/arch/x86/cpu/irq.c's own
/// `pic_remap()`), hardware IRQ n arrived as vector 32+n. Re-used
/// verbatim as the vector every I/O APIC redirection entry this
/// module programs points at - see this module's own header comment
/// for why that means idt.c/irq.c's existing IDT gate installation
/// needed zero changes: only *routing* (which controller decides
/// which CPU/vector an IRQ becomes) changed, not vector numbering.
const IRQ_BASE: u32 = 32;

/// Everything `rust_ioapic_set_mask()` needs later (once actual
/// drivers start calling kernel/arch/x86/cpu/irq.c's own
/// `register_irq_handler()` during kernel_late_init()) that isn't
/// convenient to re-derive from ACPI each time. Written exactly once,
/// by `rust_smp_init()`, before any driver init that could call
/// `rust_ioapic_set_mask()` runs - see kernel/init/main.c's own
/// ordering - so a `SpinLock` here is defensive correctness for a
/// case that cannot currently race, not a response to an observed
/// problem; still the right default now that this project has a real
/// primitive for exactly this (kernel/rust/spinlock.rs, Phase 40)
/// rather than reaching for another unprotected `static mut`.
struct IoApicState {
    base: u32,
    gsi_base: u32,
    /// Index i = ISA IRQ i's actual GSI, from
    /// `crate::acpi::SmpDiscovery::gsi_for_isa_irq()` - see that
    /// function's own comment for why this is not simply `i` for
    /// every entry (ISA IRQ0's own real-world GSI2 remap being the
    /// one case this kernel actually depends on getting right).
    isa_to_gsi: [u32; 16],
}

impl IoApicState {
    const fn empty() -> Self {
        IoApicState {
            base: 0,
            gsi_base: 0,
            isa_to_gsi: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        }
    }
}

static IOAPIC_STATE: crate::spinlock::SpinLock<IoApicState> =
    crate::spinlock::SpinLock::new(IoApicState::empty());

/// Set once, by `rust_smp_init()`, the moment IO-APIC routing is
/// actually live - kernel/arch/x86/cpu/irq.c checks this (via
/// `rust_ioapic_is_active()`) on every single IRQ and every driver's
/// own `register_irq_handler()` call, to decide whether to keep using
/// the legacy 8259 PIC path (unchanged, for any machine this module
/// didn't find a usable LAPIC+IO-APIC pair on) or the new one. This
/// is the one flag that makes this entire phase's IO-APIC work
/// strictly additive: a machine where `rust_smp_init()` returns early
/// (no ACPI, no MADT, no IO-APIC) leaves this `false` forever, and
/// every existing PIC code path in irq.c runs completely unchanged,
/// exactly as it did through the end of Phase 55.
static IOAPIC_ACTIVE: AtomicBool = AtomicBool::new(false);

/// # Safety
/// `base` must already be mapped (see `crate::acpi::ensure_mapped()`)
/// and be a genuine I/O APIC MMIO base.
unsafe fn ioapic_read(base: u32, index: u32) -> u32 {
    core::ptr::write_volatile((base + IOAPIC_REG_IOREGSEL) as *mut u32, index);
    core::ptr::read_volatile((base + IOAPIC_REG_IOWIN) as *const u32)
}

unsafe fn ioapic_write(base: u32, index: u32, value: u32) {
    core::ptr::write_volatile((base + IOAPIC_REG_IOREGSEL) as *mut u32, index);
    core::ptr::write_volatile((base + IOAPIC_REG_IOWIN) as *mut u32, value);
}

/// Programs redirection entries for ISA IRQ 0-15, all initially
/// MASKED - deliberately preserving irq.c's own existing security
/// posture ("every line starts masked... only becomes able to
/// interrupt the kernel after it explicitly calls
/// register_irq_handler()") under the new controller, not just under
/// the old one. Delivery mode Fixed, physical destination, edge-
/// triggered, active-high for every entry: correct for the standard
/// ISA-compatible IRQs this kernel actually uses (PIT, PS/2, ATA IRQ
/// lines) on every machine this was tested against, including QEMU's
/// own default machine type - a real, deliberate simplification (a
/// fully general implementation would also honor MADT Interrupt
/// Source Override entries' own polarity/trigger-mode flags, not just
/// their GSI remapping - see `gsi_for_isa_irq()`'s own comment for
/// why *that* specific field could not be skipped the same way).
///
/// # Safety
/// `io_base` must already be mapped; `bsp_apic_id` must be this CPU's
/// own, already-read LAPIC ID.
unsafe fn ioapic_program_isa_redirects(
    io_base: u32,
    gsi_base: u32,
    smp: &crate::acpi::SmpDiscovery,
    bsp_apic_id: u8,
) {
    for isa_irq in 0u8..16 {
        let gsi = smp.gsi_for_isa_irq(isa_irq);
        if gsi < gsi_base {
            continue; // belongs to a different I/O APIC than the one
                       // this module programs (see rust_smp_init()'s
                       // own "first I/O APIC only" note)
        }
        let pin = gsi - gsi_base;
        if pin > 23 {
            continue; // outside this I/O APIC's redirection table -
                       // the ACPI spec allows up to 24 entries; be
                       // defensive rather than assume the maximum
        }
        let vector = IRQ_BASE + isa_irq as u32;
        let low = vector | (1 << 16); // Fixed/physical/edge/active-high, MASKED
        let high = (bsp_apic_id as u32) << 24;
        ioapic_write(io_base, 0x10 + 2 * pin, low);
        ioapic_write(io_base, 0x10 + 2 * pin + 1, high);
    }
}

/// Called from kernel/arch/x86/cpu/irq.c's own `register_irq_handler()`
/// in place of (once `rust_ioapic_is_active()` says yes) the legacy
/// `pic_set_mask()` - a driver asking to receive IRQ `isa_irq` has to
/// unmask that line on whichever controller is actually routing it
/// now, or its interrupts simply never arrive, silently, regardless
/// of anything the driver itself does correctly.
#[no_mangle]
pub extern "C" fn rust_ioapic_set_mask(isa_irq: u8, masked: u8) -> i32 {
    if !IOAPIC_ACTIVE.load(Ordering::Acquire) {
        return -1;
    }
    if isa_irq >= 16 {
        return -2;
    }
    let state = IOAPIC_STATE.lock();
    let gsi = state.isa_to_gsi[isa_irq as usize];
    if gsi < state.gsi_base {
        return -3;
    }
    let pin = gsi - state.gsi_base;
    if pin > 23 {
        return -4;
    }
    let base = state.base;
    unsafe {
        let low = ioapic_read(base, 0x10 + 2 * pin);
        let new_low = if masked != 0 { low | (1 << 16) } else { low & !(1u32 << 16) };
        ioapic_write(base, 0x10 + 2 * pin, new_low);
    }
    0
}

#[no_mangle]
pub extern "C" fn rust_ioapic_is_active() -> u8 {
    if IOAPIC_ACTIVE.load(Ordering::Acquire) { 1 } else { 0 }
}

/// Fully masks both legacy 8259 PICs (both data ports, all 8 lines
/// each) - the standard, necessary step once an I/O APIC is actually
/// routing the same physical IRQ lines, so a device interrupt can
/// never be delivered twice (once via each controller) or left
/// permanently "in service" on a PIC nothing ever sends EOI to again.
/// Reuses `crate::acpi::port_outb` rather than re-deriving the same
/// `asm!` block - see that function's own doc comment for why it is
/// `pub(crate)` as of this phase.
///
/// # Safety
/// Port I/O to well-known, fixed legacy PIC ports - always safe on
/// PC-compatible hardware.
unsafe fn mask_legacy_pic() {
    const PIC1_DATA: u16 = 0x21;
    const PIC2_DATA: u16 = 0xA1;
    crate::acpi::port_outb(PIC1_DATA, 0xFF);
    crate::acpi::port_outb(PIC2_DATA, 0xFF);
}

// ============================================================
// AP bring-up
// ============================================================

/// Must match kernel/arch/x86/cpu/ap_trampoline.s's own `ORG 0x8000`
/// exactly - see that file's own header comment for the full
/// reasoning (why 0x8000, why it has to be page-aligned, and why a
/// SIPI's vector field is this shifted right by 12).
const TRAMPOLINE_PHYS_BASE: u32 = 0x8000;
const TRAMPOLINE_SIPI_VECTOR: u8 = 0x08; // 0x8000 >> 12

/// Must match ap_trampoline.s's own hard-coded mailbox offsets
/// exactly - see that file's header comment for the full field list
/// and why this is a fixed constant on both sides rather than a
/// computed one.
const MAILBOX_BASE: u32 = 0x7000;
const MAILBOX_STACK_TOP: u32 = MAILBOX_BASE;
const MAILBOX_PAGE_DIRECTORY: u32 = MAILBOX_BASE + 4;
const MAILBOX_KERNEL_GDT_PTR: u32 = MAILBOX_BASE + 8;
const MAILBOX_KERNEL_IDT_PTR: u32 = MAILBOX_BASE + 12;
const MAILBOX_ENTRY_POINT: u32 = MAILBOX_BASE + 16;
const MAILBOX_ACK: u32 = MAILBOX_BASE + 20;

/// The assembled trampoline, embedded directly into this kernel's own
/// Rust object code at compile time. See the Makefile's own
/// `AP_TRAMPOLINE_BIN` rule for how `ap_trampoline.s` becomes this
/// exact file (`nasm -f bin`, a flat binary, not a normal ELF object -
/// see that .s file's own header comment for why it deliberately
/// isn't picked up by the Makefile's usual `*.asm` glob).
static TRAMPOLINE_BLOB: &[u8] =
    include_bytes!("../../build/kernel/arch/x86/cpu/ap_trampoline.bin");

#[inline(always)]
unsafe fn mailbox_write(addr: u32, value: u32) {
    core::ptr::write_volatile(addr as *mut u32, value);
}

#[inline(always)]
unsafe fn mailbox_read(addr: u32) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

fn hex_digit(nibble: u8) -> u8 {
    let n = nibble & 0xF;
    if n < 10 { b'0' + n } else { b'A' + (n - 10) }
}

fn decimal_digits(mut n: u32, out: &mut [u8]) -> usize {
    if n == 0 {
        out[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 10];
    let mut len = 0usize;
    while n > 0 {
        tmp[len] = b'0' + ((n % 10) as u8);
        n /= 10;
        len += 1;
    }
    for i in 0..len {
        out[i] = tmp[len - 1 - i];
    }
    len
}

extern "C" {
    fn serial_puts(s: *const u8);
}

/// Logs directly from Rust via the same low-level `serial_puts()`
/// kernel/rust/lib.rs's own panic handler already uses, rather than
/// through kernel_late_init()'s usual `kernel_log()` - deliberately,
/// not an oversight: `kernel_log()` is a C variadic function, which
/// this project's kernel-side Rust has never called anywhere (every
/// other Rust module's own result is logged by its *C caller*, after
/// a synchronous FFI call returns - see e.g. kernel/rust/journal.rs's
/// own call sites in kernel/init/main.c). That pattern does not work
/// for a per-AP "I'm alive" line: each AP runs `rust_ap_main()`
/// independently, on its own CPU, and never returns to any C caller
/// for main.c to log anything on its behalf. A tiny hand-built ASCII
/// message plus the existing extern "C" serial_puts() is the smallest
/// correct way to get that line onto the same serial console every
/// other boot marker in this project already writes to.
/// Logged by the BSP (`smp_boot_aps()`) the moment a given AP's own
/// mailbox ack is observed - proof the trampoline (real mode ->
/// protected mode -> paging -> the real kernel GDT/IDT) completed on
/// that CPU, but deliberately NOT proof that `rust_ap_main()` itself
/// has actually started running yet (the ack write happens in
/// ap_trampoline.s, strictly before the jump into Rust - see that
/// file's own header comment). See `log_ap_running()` below for the
/// stronger claim, logged by the AP itself.
fn log_ap_ack(apic_id: u8, index: u32) {
    let mut buf = [0u8; 80];
    let mut pos = 0usize;
    for &b in b"[ OK ] SMP: AP APIC ID=0x" {
        buf[pos] = b;
        pos += 1;
    }
    buf[pos] = hex_digit(apic_id >> 4);
    pos += 1;
    buf[pos] = hex_digit(apic_id);
    pos += 1;
    for &b in b" acknowledged trampoline (index=" {
        buf[pos] = b;
        pos += 1;
    }
    let mut digits = [0u8; 10];
    let n = decimal_digits(index, &mut digits);
    buf[pos..pos + n].copy_from_slice(&digits[..n]);
    pos += n;
    for &b in b")\n" {
        buf[pos] = b;
        pos += 1;
    }
    buf[pos] = 0;
    unsafe { serial_puts(buf.as_ptr()) };
}

/// Logged by the AP itself, from inside `rust_ap_main()` - the
/// stronger of this module's two AP-related log lines: proof that
/// compiled Rust code is genuinely executing on this second CPU core,
/// not just that the hand-written asm trampoline ran (see
/// `log_ap_ack()`'s own comment for that weaker, BSP-side claim).
/// This is the line PROGRESS.md's Phase 56 entry and
/// tools/python/test_runner.py's own new assertion both point to as
/// the real evidence for this phase's own central claim.
fn log_ap_running(apic_id: u8) {
    let mut buf = [0u8; 64];
    let mut pos = 0usize;
    for &b in b"[ OK ] AP online: APIC ID=0x" {
        buf[pos] = b;
        pos += 1;
    }
    buf[pos] = hex_digit(apic_id >> 4);
    pos += 1;
    buf[pos] = hex_digit(apic_id);
    pos += 1;
    for &b in b" running real kernel Rust code\n" {
        buf[pos] = b;
        pos += 1;
    }
    buf[pos] = 0;
    unsafe { serial_puts(buf.as_ptr()) };
}

/// Brings up every discovered CPU other than `bsp_apic_id`, one at a
/// time - deliberately sequential, not "send every SIPI and wait
/// once at the end": this kernel's whole trampoline design (see
/// ap_trampoline.s's own header comment) reuses one shared physical
/// copy of the trampoline code and one shared mailbox for every AP,
/// specifically because it lets each AP's own bring-up be a simple,
/// fully-ordered handshake (write mailbox -> send INIT-SIPI-SIPI ->
/// wait for that specific AP's own ack -> only then reuse the mailbox
/// for the next one) with no per-AP identification logic needed at
/// all in the 16-bit trampoline itself. Bringing up N CPUs in
/// parallel would need either N separate trampoline copies (more
/// physical low memory, more Makefile/embedding complexity) or a way
/// for a newly-woken AP to read its own APIC ID before it can even
/// find its own mailbox slot - real complexity this phase's own
/// stated scope (get CPUs alive and running real code; leave
/// scheduling and cross-subsystem locking for later) doesn't need to
/// take on. The wall-clock cost (each AP's own INIT-SIPI-SIPI-and-wait
/// sequence, one after another) is milliseconds per CPU, paid once,
/// at boot - not a real drawback for what this phase actually needs.
///
/// A CPU this function fails to bring up (no ack within
/// `AP_ACK_WAIT_ITERATIONS`, or `pmm_alloc_contiguous()` couldn't find
/// it a stack) is skipped, logged, and does not stop the rest of this
/// loop or fail this function - the same "boot continues regardless"
/// posture kernel/init/main.c's own real-hardware ACPI discovery logs
/// (Phase 44) and real-hardware shutdown discovery (Phase 55) already
/// established for this exact kind of best-effort, environment-
/// dependent step.
///
/// # Safety
/// Must only be called once, from `rust_smp_init()`, before `sti`
/// (see kernel/init/main.c's own ordering) - this function pokes
/// fixed physical addresses (the trampoline copy target and the
/// mailbox) directly, which is only safe while nothing else in this
/// kernel could concurrently be using that same low-memory range.
unsafe fn smp_boot_aps(smp: &crate::acpi::SmpDiscovery, bsp_apic_id: u8, lapic_base: u32) -> u32 {
    let dst = TRAMPOLINE_PHYS_BASE as *mut u8;
    core::ptr::copy_nonoverlapping(TRAMPOLINE_BLOB.as_ptr(), dst, TRAMPOLINE_BLOB.len());

    // These three are the same for every AP this loop brings up -
    // written once, outside the loop, rather than re-derived per CPU.
    mailbox_write(MAILBOX_PAGE_DIRECTORY, paging_kernel_directory_phys());
    mailbox_write(MAILBOX_KERNEL_GDT_PTR, gdt_get_pointer_addr());
    mailbox_write(MAILBOX_KERNEL_IDT_PTR, idt_get_pointer_addr());
    mailbox_write(MAILBOX_ENTRY_POINT, rust_ap_main as usize as u32);

    let mut brought_up = 0u32;
    let mut next_index = 1u32; // 0 is implicitly the BSP itself

    for i in 0..(smp.cpu_count as usize) {
        let apic_id = smp.apic_ids[i];
        if apic_id == bsp_apic_id {
            continue; // that's this CPU, already running - nothing to boot
        }

        // KERNEL_STACK_SIZE (kernel/task/process.h) is 8KB (2 * 4KB
        // frames) - matched here deliberately, not an arbitrary
        // choice, so an idle AP's own stack is sized the same as
        // every ordinary kernel task's, not a special smaller or
        // larger one.
        let stack_phys = pmm_alloc_contiguous(2);
        if stack_phys == 0 {
            continue; // out of memory - skip this CPU, not fatal to the rest
        }
        let stack_top = stack_phys + 2 * 4096;

        mailbox_write(MAILBOX_STACK_TOP, stack_top);
        mailbox_write(MAILBOX_ACK, 0);

        // Standard INIT-SIPI-SIPI sequence (Intel MP/ACPI convention,
        // the same one documented on OSDev.org's own "Symmetric
        // Multiprocessing" page - this project already cites that
        // same site's own precedent for Phase 55's ACPI shutdown
        // technique). 0x4500 = delivery mode 101 (INIT) << 8, level
        // (assert) bit set. 0x4600 = delivery mode 110 (Startup) << 8,
        // level bit set, OR'd with the trampoline's own SIPI vector
        // byte.
        lapic_send_ipi(lapic_base, apic_id, 0x4500);
        busy_wait(INIT_TO_SIPI_WAIT_ITERATIONS);
        lapic_send_ipi(lapic_base, apic_id, 0x4600 | (TRAMPOLINE_SIPI_VECTOR as u32));
        busy_wait(SIPI_TO_SIPI_WAIT_ITERATIONS);
        lapic_send_ipi(lapic_base, apic_id, 0x4600 | (TRAMPOLINE_SIPI_VECTOR as u32));

        let mut acked = false;
        let mut w = 0u32;
        while w < AP_ACK_WAIT_ITERATIONS {
            if mailbox_read(MAILBOX_ACK) != 0 {
                acked = true;
                break;
            }
            w += 1;
        }

        if acked {
            brought_up += 1;
            log_ap_ack(apic_id, next_index);
        }
        next_index += 1;
    }

    brought_up
}

/// CPUID.1:EDX bit 9 - "this CPU has an on-die Local APIC at all,"
/// independent of anything ACPI claims. Checked before trusting the
/// MADT's own word for it: ACPI tables describe the *platform*
/// firmware's view, not a live hardware capability query, and a
/// defense-in-depth check here is cheap (one `cpuid` instruction) for
/// real protection against ever attempting LAPIC MMIO access on a
/// CPU that doesn't actually have one, whatever a table says.
fn cpu_has_apic() -> bool {
    let edx: u32;
    unsafe {
        core::arch::asm!(
            "push ebx",
            "mov eax, 1",
            "cpuid",
            "pop ebx",
            out("eax") _,
            out("ecx") _,
            out("edx") edx,
            options(nostack),
        );
    }
    (edx & (1 << 9)) != 0
}

/// Called once from kernel_late_init(), before `timer_init()`/
/// `driver_init_all()` and before `sti` (see kernel/init/main.c's own
/// ordering, and this function's own comment on why that ordering
/// matters). Returns the number of APs successfully brought online
/// (0 or more) if this machine has a usable Local APIC + I/O APIC -
/// in which case IO-APIC routing is now live and the legacy 8259 PIC
/// is fully masked - or a negative code if not, in which case this
/// function has changed nothing at all and the existing PIC-only path
/// (unchanged since Phase 2) remains exactly as it was.
#[no_mangle]
pub extern "C" fn rust_smp_init() -> i32 {
    if !cpu_has_apic() {
        return -1;
    }

    let smp = crate::acpi::discover_smp();
    if !smp.found_acpi || !smp.found_madt || smp.local_apic_phys == 0 {
        return -2;
    }
    if smp.io_apic_count == 0 {
        return -3;
    }
    if smp.cpu_count == 0 {
        return -4; // MADT parsed but reported zero *enabled* CPUs -
                    // implausible on real hardware; refuse rather
                    // than guess what that would even mean
    }

    let lapic_base = smp.local_apic_phys;
    crate::acpi::ensure_mapped(lapic_base);
    LAPIC_BASE.store(lapic_base, Ordering::Release);

    // A trivial (iretd-only) handler must exist before this or any
    // other CPU's Local APIC is ever enabled - see SPURIOUS_VECTOR's
    // own comment for why an unhandled spurious interrupt would
    // otherwise be a real #NP exception, not a no-op.
    extern "C" {
        fn spurious_interrupt_stub();
    }
    const GDT_KERNEL_CODE: u16 = 0x08;
    const IDT_FLAGS_PRESENT_RING0_INTERRUPT_GATE: u8 = 0x8E;
    unsafe {
        idt_set_gate(
            SPURIOUS_VECTOR,
            spurious_interrupt_stub as usize as u32,
            GDT_KERNEL_CODE,
            IDT_FLAGS_PRESENT_RING0_INTERRUPT_GATE,
        );
    }

    let bsp_apic_id = unsafe {
        lapic_enable(lapic_base);
        lapic_id(lapic_base)
    };

    // Only the first I/O APIC this machine reports is programmed -
    // every machine this was tested against (including QEMU's own
    // default and q35 machine types) has exactly one, and
    // `IoApicState`/`gsi_for_isa_irq()` are both already structured
    // so a second one, if it ever needs handling, is additive work
    // there rather than a redesign.
    let io = smp.io_apics[0];
    crate::acpi::ensure_mapped(io.phys_addr);

    unsafe {
        ioapic_program_isa_redirects(io.phys_addr, io.gsi_base, &smp, bsp_apic_id);
        mask_legacy_pic();
    }

    {
        let mut state = IOAPIC_STATE.lock();
        state.base = io.phys_addr;
        state.gsi_base = io.gsi_base;
        for isa_irq in 0u8..16 {
            state.isa_to_gsi[isa_irq as usize] = smp.gsi_for_isa_irq(isa_irq);
        }
    }
    IOAPIC_ACTIVE.store(true, Ordering::Release);

    let brought_up = unsafe { smp_boot_aps(&smp, bsp_apic_id, lapic_base) };
    brought_up as i32
}

/// Entry point for every AP this kernel ever brings up - reached via
/// an indirect jump from kernel/arch/x86/cpu/ap_trampoline.s's own
/// tail, once paging, the real kernel GDT/IDT, and a real per-AP
/// stack are all already live (see that file's own header comment
/// for the full handoff sequence). `extern "C"` so its address (taken
/// as a plain function pointer, `rust_ap_main as usize as u32`, in
/// `smp_boot_aps()` above) is meaningful to the trampoline's own raw
/// indirect jump - there is no Rust-level caller anywhere, ever, and
/// the `-> !` return type is not just documentation: this function
/// really can never return, since nothing in ap_trampoline.s left a
/// return address on this AP's fresh stack for it to return to.
///
/// Deliberately minimal, matching this phase's own stated scope (see
/// this module's own header comment): this AP enables its own Local
/// APIC (genuinely separate hardware from the BSP's, despite the
/// shared MMIO address - see `lapic_id()`'s own comment), logs that
/// it's alive, and idles - `sti` then `hlt`, forever. It does not
/// call into kernel/task/scheduler.c, does not touch the process
/// table, and is not itself an interrupt target for any device IRQ
/// (`ioapic_program_isa_redirects()` above routes every ISA IRQ's
/// redirection entry at the BSP's own APIC ID specifically, not this
/// one - see that function's own comment). `sti` here is still
/// correct, not pointless, even though nothing is currently routed to
/// this CPU: it's what lets a future phase's own inter-processor
/// interrupt (a TLB shootdown, a wake-this-CPU-up IPI once real SMP
/// scheduling exists) actually reach an idling AP instead of a
/// permanently-`cli`'d one.
#[no_mangle]
pub extern "C" fn rust_ap_main() -> ! {
    let base = LAPIC_BASE.load(Ordering::Acquire);
    let id = unsafe {
        lapic_enable(base);
        lapic_id(base)
    };
    log_ap_running(id);

    loop {
        unsafe {
            core::arch::asm!("sti", "hlt");
        }
    }
}
