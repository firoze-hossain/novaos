#ifndef ARCH_X86_CPU_IDT_H
#define ARCH_X86_CPU_IDT_H

#include "../../../include/types.h"

/* Sets up all 256 IDT entries: gates 0-31 for CPU exceptions (see isr.c),
 * gates 32-47 for the remapped hardware IRQs (see irq.c), and loads the
 * table with LIDT. Must run after gdt_init(). */
void idt_init(void);

/* Used internally by isr.c / irq.c to register a raw gate. Exposed here
 * (rather than made static) because both files populate the same table. */
void idt_set_gate(uint8_t num, uint32_t base, uint16_t selector, uint8_t flags);

/* Phase 56: the physical address of this kernel's own already-built
 * idt_pointer struct - see gdt.h's own gdt_get_pointer_addr() for the
 * identical reasoning, one level down (IDT instead of GDT). */
uint32_t idt_get_pointer_addr(void);

#endif
