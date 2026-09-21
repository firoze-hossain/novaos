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
    /* cs reveals which ring the fault occurred in (0x8 = kernel,
     * 0x1B = user) - useresp/ss are deliberately not printed here:
     * the CPU only pushes them for a real ring3->ring0 transition, so
     * for a ring0-originating fault (cs=0x8) they're stale, unrelated
     * stack bytes this fixed-size registers_t struct still "reads"
     * anyway - genuinely useful information the first time this was
     * investigated (confirmed a fault was a real ring0 exception, not
     * corrupted execution, by checking cs), not something to keep
     * printing misleadingly. */
    kernel_log("[DIAG] full state: cs=0x%x eflags=0x%x eax=0x%x ebx=0x%x "
               "ecx=0x%x edx=0x%x\n",
               (int)regs->cs, (int)regs->eflags, (int)regs->eax,
               (int)regs->ebx, (int)regs->ecx, (int)regs->edx);
    {
        extern void* scheduler_current(void);
        struct fault_diag_process { int pid; char name[32]; };
        struct fault_diag_process* cur =
            (struct fault_diag_process*)scheduler_current();
        if (cur != NULL) {
            kernel_log("[DIAG] fault occurred while running process "
                       "'%s' (pid %d)\n", cur->name, cur->pid);
        } else {
            kernel_log("[DIAG] fault occurred with no current process "
                       "(pre-scheduler)\n");
        }
    }

    /* Phase 54: this exception handler already has the one thing a
     * software-detected kernel_panic() call never does - the CPU's own
     * real register state at the exact moment of the fault - so it
     * uses kernel_panic_fault() directly instead of the plain
     * kernel_panic() wrapper. See kernel/rust/crashdump.rs's own
     * header comment for what that state is used for. No faulting
     * address here (that's page-fault-specific, CR2 - see kernel/arch/
     * x86/mm/paging.c's own page_fault_handler(), which registers its
     * own handler for vector 14 and never reaches this generic path). */
    kernel_panic_fault(exception_names[regs->int_no], regs, false, 0);
}
