#ifndef ARCH_X86_CPU_SYSCALL_H
#define ARCH_X86_CPU_SYSCALL_H

#include "isr.h"

#define SYSCALL_VECTOR 0x80

/* Convention (deliberately simple, not Linux-ABI-compatible): syscall
 * number in EAX, up to one argument in EBX. No return value is passed
 * back yet - every current syscall is fire-and-forget. */
#define SYS_WRITE 1 /* EBX = pointer to a NUL-terminated string to print */
#define SYS_EXIT  2 /* terminates the calling process; never returns    */
#define SYS_YIELD 3 /* voluntarily gives up the rest of this quantum    */

/* Installs the int 0x80 gate with DPL=3 (required for ring-3 code to
 * invoke it via the INT instruction at all - the CPU checks CPL <= gate
 * DPL for software interrupts) and points it at the dedicated syscall
 * entry stub (see syscall_stub.asm). */
void syscall_init(void);

#endif
