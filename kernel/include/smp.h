#ifndef INCLUDE_SMP_H
#define INCLUDE_SMP_H

#include "types.h"

/* Phase 57 / Phase 95: how many CPUs this kernel's own per-CPU scheduler/TSS
 * arrays (kernel/task/scheduler.c's `current[]`, kernel/arch/x86/cpu/
 * tss.c's `tss[]`) are sized for - expanded from 4 to 8 in Phase 95 for
 * SMP scaling validation beyond the 2-CPU test default. Supports up to 8
 * logical cores (1 BSP + 7 APs) under QEMU `-smp 4` or `-smp 8` and real
 * multi-core hardware.
 *
 * kernel/rust/apic.rs mirrors this exact value under the same name
 * (SCHED_MAX_CPUS) - duplicated rather than shared via a header, this
 * project's established FFI convention for a small constant that has
 * to be identical on both sides of the Rust/C boundary (see e.g. the
 * AP trampoline mailbox offsets duplicated between that file and
 * kernel/arch/x86/cpu/ap_trampoline.s). If this is ever changed, both
 * copies must change together.
 *
 * A CPU beyond this bound (only reachable via a manual `-smp N` well
 * past this project's own tested default) still boots and idles
 * safely - kernel/rust/apic.rs's own smp_boot_aps() stops bringing up
 * further APs once this many are already registered, and
 * rust_smp_current_cpu_index() below fails safe (0xFF) for anything
 * that slips through - it just never joins the scheduler. */
#define SCHED_MAX_CPUS 8

/* Returns a small, stable 0-based index for whichever CPU calls this
 * (0 = the BSP, 1.. = each AP in bring-up order) - implemented in
 * kernel/rust/apic.rs, which is the natural place for this: it
 * already computes a real x86 APIC id for every CPU it brings up (for
 * IPI targeting), and this function's own real lookup key (the CPUID
 * "initial APIC ID", CPUID.1:EBX[31:24]) needs nothing but the
 * `cpuid` instruction - no LAPIC MMIO mapping required, so this is
 * safe to call from EVERY CPU on EVERY machine, including a plain
 * single-core one with no usable Local APIC/IO APIC pair at all
 * (kernel/rust/apic.rs's rust_smp_init() registers CPU 0 this way
 * unconditionally, before any of its own SMP-capability checks can
 * bail out early - see that function's own comment). Every caller in
 * this kernel (kernel/task/scheduler.c, kernel/task/process.c) that
 * uses the returned index to index a `SCHED_MAX_CPUS`-sized array
 * MUST check for 0xFF (or >= SCHED_MAX_CPUS) first and fail safe
 * (never schedule/never touch per-CPU state) rather than index past
 * the array - see scheduler.c's own do_schedule() for the pattern. */
uint8_t rust_smp_current_cpu_index(void);

#endif
