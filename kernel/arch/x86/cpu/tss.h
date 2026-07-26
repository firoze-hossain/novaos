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
 * There is exactly one TSS, loaded once at boot; the scheduler updates
 * its esp0 field (via tss_set_kernel_stack()) every time it switches
 * to a different process, so esp0 always points at *that* process's
 * own kernel stack. */
void tss_init(void);

void tss_set_kernel_stack(uint32_t esp0);

#endif
