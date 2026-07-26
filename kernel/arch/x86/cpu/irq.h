#ifndef ARCH_X86_CPU_IRQ_H
#define ARCH_X86_CPU_IRQ_H

#include "isr.h"

/* After PIC remapping, hardware IRQ n arrives as interrupt vector 32+n. */
#define IRQ_BASE 32

/* Installs IDT gates 32-47, remaps the 8259 PIC so hardware IRQs don't
 * collide with CPU exception vectors 0-31, and masks every line except
 * the ones a driver has actually registered a handler for. */
void irq_install_gates(void);

/* Registers `handler` to run whenever IRQ `irq` (0-15) fires, and
 * unmasks that line on the PIC. Called by timer_init(), keyboard_init(). */
void register_irq_handler(uint8_t irq, isr_handler_t handler);

#endif
