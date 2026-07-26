#ifndef ARCH_X86_CPU_ISR_H
#define ARCH_X86_CPU_ISR_H

#include "../../../include/types.h"

/* Snapshot of CPU state at the moment an interrupt fired, in the exact
 * order isr_stubs.asm pushes it. Handlers (and later, the scheduler's
 * context switch) read/write this to inspect or modify the interrupted
 * task's state. */
typedef struct registers {
    uint32_t ds;
    uint32_t edi, esi, ebp, esp_dummy, ebx, edx, ecx, eax; /* pusha order */
    uint32_t int_no, err_code;
    uint32_t eip, cs, eflags, useresp, ss;
} registers_t;

typedef void (*isr_handler_t)(registers_t* regs);

/* Installs IDT gates 0-31 pointing at the isr0..isr31 asm stubs. */
void isr_install_gates(void);

/* Called from isr_common_stub (isr_stubs.asm) for every exception. */
void isr_handler(registers_t* regs);

/* Lets higher-level code (currently just kernel_panic on fatal faults)
 * override the default behaviour for a specific exception vector. */
void register_exception_handler(uint8_t vector, isr_handler_t handler);

#endif
