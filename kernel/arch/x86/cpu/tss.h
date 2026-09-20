#ifndef ARCH_X86_CPU_TSS_H
#define ARCH_X86_CPU_TSS_H

#include "../../../include/types.h"

/* NovaOS does software (not hardware) task switching - see
 * context_switch.asm - so almost every field in the TSS is unused. The
 * one field that matters is esp0/ss0: when the CPU takes an interrupt
 * or executes `int 0x80` while running in ring 3, it looks up esp0/ss0
 * in the *currently loaded* TSS to know which ring-0 stack to switch
 * to before pushing the interrupt frame. Without this, an interrupt
 * firing while in ring 3 uses garbage as a kernel stack pointer -
 * simplest case is an immediate second (double) fault.
 *
 * Phase 57: previously exactly one TSS, loaded once at boot, shared by
 * whatever single CPU this kernel ever ran on. Real SMP needs one *per
 * CPU*: two cores genuinely running different ring-3 processes at the
 * same physical instant each need their own, independent esp0 (this
 * CPU's own currently-scheduled process's kernel stack) - one shared
 * field would have one CPU's ring3->ring0 transition silently load the
 * *other* CPU's kernel stack pointer, corrupting both. tss_init() now
 * installs SCHED_MAX_CPUS separate TSS descriptors into the (still
 * single, shared) GDT in one BSP-only call at boot (see gdt.h's own
 * GDT_TSS_GATE_INDEX()/GDT_TSS_SELECTOR()); every CPU that will ever
 * take a ring3->ring0 transition - the BSP and every AP - then loads
 * its own with tss_load_this_cpu(), and the scheduler updates only its
 * own slot via the now cpu-indexed tss_set_kernel_stack(). */
void tss_init(void);

/* Loads (LTR) this CPU's own TSS selector. Must be called once by
 * every CPU that will ever take a ring3->ring0 transition - the BSP
 * (kernel/init/main.c's kernel_early_init(), right after tss_init())
 * and every AP (kernel/rust/apic.rs's rust_ap_main(), once it knows
 * its own scheduler CPU index) - always *after* tss_init() has already
 * installed every CPU's descriptor into the shared GDT. */
void tss_load_this_cpu(uint8_t cpu_index);

/* Updates cpu_index's own esp0 field only - never any other CPU's.
 * `cpu_index >= SCHED_MAX_CPUS` is silently ignored (fail safe; every
 * real caller already validated its own index via
 * rust_smp_current_cpu_index() before reaching here - see
 * kernel/task/scheduler.c). */
void tss_set_kernel_stack(uint8_t cpu_index, uint32_t esp0);

#endif
