/*
 * syscall.c - int 0x80 dispatch
 *
 * This is the ONLY sanctioned way ring-3 code talks to the kernel - a
 * user task has no other path to VGA output, process control, or
 * anything else privileged, which is the actual security property
 * ring 3 exists to provide (see kernel/task/user_demo.c: it really
 * can't just call vga_puts() directly - that's ring-0-only code, and
 * calling it from ring 3 would fault, not merely be bad practice).
 */
#include "syscall.h"
#include "idt.h"
#include "gdt.h"
#include "../../drivers/vga/vga.h"
#include "../../task/process.h"
#include "../../task/scheduler.h"
#include "../../include/kernel.h"

extern void isr128(void);

void syscall_init(void) {
    /* 0xEE = present, ring 3 DPL, 32-bit interrupt gate. The DPL is
     * what actually matters here: the CPU checks CPL <= gate DPL for
     * a software interrupt raised via INT, so a ring-3-only gate
     * (DPL=0, like every other IDT entry) would take a #GP fault the
     * instant user code tried `int 0x80` - this is the one interrupt
     * vector deliberately opened up to ring 3. */
    idt_set_gate(SYSCALL_VECTOR, (uint32_t)isr128, GDT_KERNEL_CODE, 0xEE);
}

void syscall_handler(registers_t* regs) {
    switch (regs->eax) {
        case SYS_WRITE: {
            const char* str = (const char*)regs->ebx;
            vga_puts(str);
            process_t* p = process_current();
            kernel_log("[SYSCALL] SYS_WRITE from pid %d ('%s')\n",
                       p != NULL ? p->pid : -1, str);
            break;
        }

        case SYS_EXIT:
            process_exit_current(); /* never returns */
            break;

        case SYS_YIELD:
            scheduler_yield();
            break;

        default:
            kernel_log("[WARN] Unknown syscall number %d\n", (int)regs->eax);
            break;
    }
}
