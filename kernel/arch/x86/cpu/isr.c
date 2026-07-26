/*
 * isr.c - CPU exception dispatch (interrupt vectors 0-31)
 *
 * An unhandled CPU exception on real hardware (or in the original
 * Phase 1 kernel) means the machine silently triple-faults and reboots
 * with no explanation. That is unacceptable for a security-conscious
 * OS: NovaOS instead decodes the fault, logs full register state to
 * the serial console, and stops cleanly via kernel_panic() so a
 * developer (or `make debug`) can see exactly what went wrong.
 */
#include "isr.h"
#include "idt.h"
#include "gdt.h"
#include "../../../include/kernel.h"

/* isr0..isr31 are the raw asm entry points defined in isr_stubs.asm. */
extern void isr0(void);  extern void isr1(void);  extern void isr2(void);
extern void isr3(void);  extern void isr4(void);  extern void isr5(void);
extern void isr6(void);  extern void isr7(void);  extern void isr8(void);
extern void isr9(void);  extern void isr10(void); extern void isr11(void);
extern void isr12(void); extern void isr13(void); extern void isr14(void);
extern void isr15(void); extern void isr16(void); extern void isr17(void);
extern void isr18(void); extern void isr19(void); extern void isr20(void);
extern void isr21(void); extern void isr22(void); extern void isr23(void);
extern void isr24(void); extern void isr25(void); extern void isr26(void);
extern void isr27(void); extern void isr28(void); extern void isr29(void);
extern void isr30(void); extern void isr31(void);

static isr_handler_t exception_handlers[32];

static const char* exception_names[32] = {
    "Division By Zero",       "Debug",
    "Non Maskable Interrupt", "Breakpoint",
    "Into Detected Overflow", "Out of Bounds",
    "Invalid Opcode",         "No Coprocessor",
    "Double Fault",           "Coprocessor Segment Overrun",
    "Bad TSS",                "Segment Not Present",
    "Stack Fault",            "General Protection Fault",
    "Page Fault",             "Unknown Interrupt",
    "Coprocessor Fault",      "Alignment Check",
    "Machine Check",          "SIMD Floating-Point",
    "Reserved",                "Reserved",
    "Reserved",                "Reserved",
    "Reserved",                "Reserved",
    "Reserved",                "Reserved",
    "Reserved",                "Reserved",
    "Reserved",                "Reserved",
};

void register_exception_handler(uint8_t vector, isr_handler_t handler) {
    if (vector < 32) {
        exception_handlers[vector] = handler;
    }
}

void isr_install_gates(void) {
    /* 0x8E = present, ring 0, 32-bit interrupt gate. Interrupt gates
     * (as opposed to trap gates) clear IF, so a second interrupt can't
     * interrupt our fault handler while it's still deciding what to do. */
    const uint8_t flags = 0x8E;

    void (*stubs[32])(void) = {
        isr0,  isr1,  isr2,  isr3,  isr4,  isr5,  isr6,  isr7,
        isr8,  isr9,  isr10, isr11, isr12, isr13, isr14, isr15,
        isr16, isr17, isr18, isr19, isr20, isr21, isr22, isr23,
        isr24, isr25, isr26, isr27, isr28, isr29, isr30, isr31,
    };

    for (int i = 0; i < 32; i++) {
        idt_set_gate((uint8_t)i, (uint32_t)stubs[i], GDT_KERNEL_CODE, flags);
    }
}

void isr_handler(registers_t* regs) {
    if (regs->int_no < 32 && exception_handlers[regs->int_no]) {
        exception_handlers[regs->int_no](regs);
        return;
    }

    kernel_log("[FAULT] %s (vector %d, error code 0x%x) at eip=0x%x\n",
               exception_names[regs->int_no], (int)regs->int_no,
               (int)regs->err_code, (int)regs->eip);

    kernel_panic(exception_names[regs->int_no]);
}
