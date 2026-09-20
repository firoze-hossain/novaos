#ifndef ARCH_X86_CPU_GDT_H
#define ARCH_X86_CPU_GDT_H

#include "../../../include/types.h"
#include "../../../include/smp.h"

/* Segment selectors. Ring 3 (user) selectors are defined now, ahead of
 * the userspace/process work in Phase 4, so the scheduler and syscall
 * entry code don't need to touch the GDT layout later. */
#define GDT_KERNEL_CODE 0x08
#define GDT_KERNEL_DATA 0x10
#define GDT_USER_CODE   (0x18 | 3)
#define GDT_USER_DATA   (0x20 | 3)

/* Phase 57: previously exactly one fixed TSS descriptor (gate 5,
 * selector 0x28) shared by whatever single CPU this kernel ever ran
 * on. Real SMP needs one TSS *per CPU* (see tss.h's own updated
 * comment for why), so gates 5..5+SCHED_MAX_CPUS-1 are now each CPU's
 * own descriptor, all installed into this one shared GDT by a single
 * BSP-only tss_init() call at boot, before any AP exists - every CPU
 * still shares the same GDT (this kernel has exactly one, as before),
 * it just now has more than one TSS-shaped entry in it to choose from.
 * GDT_TSS_SELECTOR(0) is exactly the old fixed 0x28 value, so the BSP
 * ends up loading the identical selector it always did. */
#define GDT_TSS_GATE_INDEX(cpu) (5 + (cpu))
#define GDT_TSS_SELECTOR(cpu) ((uint16_t)(GDT_TSS_GATE_INDEX(cpu) * 8))

void gdt_init(void);

/* Exposed so tss.c can install the TSS descriptor into GDT entry 5
 * without gdt.c needing to know anything about what a TSS is. */
void gdt_set_gate(int32_t num, uint32_t base, uint32_t limit,
                   uint8_t access, uint8_t gran);

/* Phase 56: the physical address of this kernel's own already-built
 * gdt_pointer struct (the exact {limit:u16, base:u32} packed format
 * LGDT expects). Nothing before this phase ever needed this from
 * outside gdt.c - kernel/arch/x86/cpu/ap_trampoline.s's own mailbox
 * carries this value so a newly-woken secondary CPU can LGDT the
 * *same*, already-initialized table the BSP uses (this kernel has
 * exactly one GDT, shared by every CPU, not rebuilt per-CPU) instead
 * of needing gdt_init() re-run (and re-racing gdt_flush()'s own
 * segment-register reload) on each one. */
uint32_t gdt_get_pointer_addr(void);

#endif
