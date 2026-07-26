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

void gdt_init(void);

#endif
