/*
 * idt.c - Interrupt Descriptor Table
 *
 * The IDT tells the CPU which function to jump to for each of the 256
 * possible interrupt/exception vectors. NovaOS splits vector ownership
 * as follows:
 *   0-31   CPU exceptions (divide-by-zero, page fault, GPF, ...)  -> isr.c
 *   32-47  Hardware IRQs, remapped from the PIC's default 8-15/0-7 -> irq.c
 *   48-255 Reserved (future syscall gate, e.g. int 0x80, goes here)
 */
#include "idt.h"
#include "isr.h"
#include "irq.h"
#include "gdt.h"

struct idt_entry {
    uint16_t base_low;
    uint16_t selector;
    uint8_t  always0;
    uint8_t  flags;
    uint16_t base_high;
} __attribute__((packed));

struct idt_ptr {
    uint16_t limit;
    uint32_t base;
} __attribute__((packed));

#define IDT_ENTRIES 256

static struct idt_entry idt[IDT_ENTRIES];
static struct idt_ptr   idt_pointer;

/* Implemented in gdt_flush.asm's sibling, idt_flush (see isr_stubs.asm). */
extern void idt_flush(uint32_t idt_ptr_addr);

void idt_set_gate(uint8_t num, uint32_t base, uint16_t selector, uint8_t flags) {
    idt[num].base_low  = (uint16_t)(base & 0xFFFF);
    idt[num].base_high = (uint16_t)((base >> 16) & 0xFFFF);
    idt[num].selector  = selector;
    idt[num].always0   = 0;
    idt[num].flags     = flags;
}

void idt_init(void) {
    idt_pointer.limit = (uint16_t)(sizeof(struct idt_entry) * IDT_ENTRIES - 1);
    idt_pointer.base  = (uint32_t)&idt;

    for (int i = 0; i < IDT_ENTRIES; i++) {
        idt_set_gate((uint8_t)i, 0, 0, 0);
    }

    isr_install_gates();   /* vectors 0-31  */
    irq_install_gates();   /* vectors 32-47, plus PIC remap */

    idt_flush((uint32_t)&idt_pointer);
}
