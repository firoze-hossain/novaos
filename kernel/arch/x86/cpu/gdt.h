#ifndef ARCH_X86_CPU_GDT_H
#define ARCH_X86_CPU_GDT_H

#include "../../../include/types.h"

/* Segment selectors. Ring 3 (user) selectors are defined now, ahead of
 * the userspace/process work in Phase 4, so the scheduler and syscall
 * entry code don't need to touch the GDT layout later. */
#define GDT_KERNEL_CODE 0x08
#define GDT_KERNEL_DATA 0x10
#define GDT_USER_CODE   (0x18 | 3)
#define GDT_USER_DATA   (0x20 | 3)
#define GDT_TSS_SELECTOR 0x28

void gdt_init(void);

/* Exposed so tss.c can install the TSS descriptor into GDT entry 5
 * without gdt.c needing to know anything about what a TSS is. */
void gdt_set_gate(int32_t num, uint32_t base, uint32_t limit,
                   uint8_t access, uint8_t gran);

#endif
