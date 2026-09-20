/*
 * irq.c - Hardware interrupt dispatch and 8259 PIC (re)configuration
 *
 * By default the two 8259 Programmable Interrupt Controllers fire IRQs
 * 0-7 on interrupt vectors 8-15 and IRQs 8-15 on vectors 0x70-0x77 -
 * both of which collide with (or sit awkwardly next to) the CPU
 * exception vectors isr.c owns. The standard fix, done once at boot,
 * is to reprogram the PIC to fire IRQ n on vector 32+n instead.
 *
 * Security note: every line starts masked. A driver only becomes able
 * to interrupt the kernel after it explicitly calls
 * register_irq_handler(), which is the smallest attack surface that
 * still lets Phase 2's timer and keyboard work.
 */
#include "irq.h"
#include "idt.h"
#include "gdt.h"
#include "../io.h"

#define PIC1_COMMAND 0x20
#define PIC1_DATA    0x21
#define PIC2_COMMAND 0xA0
#define PIC2_DATA    0xA1

#define PIC_EOI      0x20

/* Phase 56: kernel/rust/apic.rs's own I/O APIC driver, which - only
 * on a machine with a usable Local APIC + I/O APIC pair (see
 * rust_smp_init()'s own doc comment for exactly what "usable" checks)
 * - takes over routing every one of these same 16 ISA IRQ lines from
 * the legacy 8259 PIC this file has driven since Phase 2.
 * rust_ioapic_is_active() is what makes that switch-over safe to be
 * silently absent: on any machine rust_smp_init() didn't find both
 * pieces on, it never returns true, and this file's every existing
 * PIC code path below (pic_remap(), pic_set_mask(), the two-line EOI
 * in irq_handler()) runs completely unchanged from how Phase 2 first
 * wrote it. */
extern uint8_t rust_ioapic_is_active(void);
extern void rust_apic_send_eoi(void);
extern int rust_ioapic_set_mask(uint8_t irq, uint8_t masked);

extern void irq0(void);  extern void irq1(void);  extern void irq2(void);
extern void irq3(void);  extern void irq4(void);  extern void irq5(void);
extern void irq6(void);  extern void irq7(void);  extern void irq8(void);
extern void irq9(void);  extern void irq10(void); extern void irq11(void);
extern void irq12(void); extern void irq13(void); extern void irq14(void);
extern void irq15(void);

static isr_handler_t irq_handlers[16];

static void pic_remap(void) {
    uint8_t mask1 = inb(PIC1_DATA);
    uint8_t mask2 = inb(PIC2_DATA);

    outb(PIC1_COMMAND, 0x11); io_wait(); /* ICW1: init, expect ICW4 */
    outb(PIC2_COMMAND, 0x11); io_wait();
    outb(PIC1_DATA, 0x20);    io_wait(); /* ICW2: master offset -> 32 */
    outb(PIC2_DATA, 0x28);    io_wait(); /* ICW2: slave offset  -> 40 */
    outb(PIC1_DATA, 0x04);    io_wait(); /* ICW3: slave is on IRQ2 */
    outb(PIC2_DATA, 0x02);    io_wait(); /* ICW3: slave's cascade identity */
    outb(PIC1_DATA, 0x01);    io_wait(); /* ICW4: 8086 mode */
    outb(PIC2_DATA, 0x01);    io_wait();

    outb(PIC1_DATA, mask1);              /* restore saved masks */
    outb(PIC2_DATA, mask2);
}

static void pic_set_mask(uint8_t irq, int masked) {
    uint16_t port = (irq < 8) ? PIC1_DATA : PIC2_DATA;
    uint8_t irq_bit = irq < 8 ? irq : (uint8_t)(irq - 8);

    uint8_t value = inb(port);
    if (masked) {
        value |= (uint8_t)(1 << irq_bit);
    } else {
        value &= (uint8_t)~(1 << irq_bit);
    }
    outb(port, value);
}

void irq_install_gates(void) {
    pic_remap();

    /* Mask everything until a driver asks for a specific line. */
    outb(PIC1_DATA, 0xFF);
    outb(PIC2_DATA, 0xFF);

    const uint8_t flags = 0x8E; /* present, ring 0, 32-bit interrupt gate */
    void (*stubs[16])(void) = {
        irq0,  irq1,  irq2,  irq3,  irq4,  irq5,  irq6,  irq7,
        irq8,  irq9,  irq10, irq11, irq12, irq13, irq14, irq15,
    };

    for (int i = 0; i < 16; i++) {
        idt_set_gate((uint8_t)(IRQ_BASE + i), (uint32_t)stubs[i],
                     GDT_KERNEL_CODE, flags);
    }
}

void register_irq_handler(uint8_t irq, isr_handler_t handler) {
    if (irq >= 16) {
        return;
    }
    irq_handlers[irq] = handler;

    if (rust_ioapic_is_active()) {
        /* I/O APIC routing is live - unmask this line's own
         * redirection-table entry instead of the (now fully masked,
         * see rust_smp_init()'s own mask_legacy_pic()) PIC. There is
         * no cascade-line concept under IO-APIC routing at all (that
         * was purely an artifact of the 8259 master/slave wiring),
         * so unlike the PIC branch below, nothing extra is needed for
         * irq >= 8. */
        rust_ioapic_set_mask(irq, 0);
        return;
    }

    pic_set_mask(irq, 0);

    /* Any slave-PIC line (8-15) is physically wired through IRQ2 on
     * the master PIC - that's the cascade line, not a normal device
     * IRQ. If it stays masked, no slave interrupt can ever reach the
     * CPU at all, regardless of the slave PIC's own mask. This went
     * unnoticed through Phases 2-6 because every IRQ user until now
     * (the PIT on 0, the keyboard on 1) lived on the master PIC;
     * Phase 7's PS/2 mouse on IRQ12 was the first to need this. */
    if (irq >= 8) {
        pic_set_mask(2, 0);
    }
}

void irq_handler(registers_t* regs) {
    uint8_t irq = (uint8_t)(regs->int_no - IRQ_BASE);

    /* EOI must be sent BEFORE dispatching to the handler, not after.
     * Both controllers this kernel can route through share that same
     * requirement (a handler that triggers a context switch - see the
     * scheduler's timer tick hook, Phase 4 - can `ret` straight into a
     * brand new task's stack and never return to this call frame at
     * all), just via a different mechanism each:
     *   - The 8259 PIC (Phase 2 through Phase 55, and still today on
     *     any machine Phase 56's rust_smp_init() didn't upgrade) is in
     *     normal (non-auto) EOI mode, so an IRQ line stays "in
     *     service" until an explicit EOI command is sent to the
     *     relevant PIC's own command port - the slave PIC must be
     *     acknowledged before the master for any IRQ that arrived
     *     through the cascade (IRQ 8-15). Found the hard way,
     *     Phase 4: the very first scheduler tick worked once and then
     *     no further timer interrupt ever fired again.
     *   - A Local APIC (Phase 56, once rust_ioapic_is_active())
     *     instead needs a single write to *this CPU's own* LAPIC EOI
     *     register - kernel/rust/apic.rs's own rust_apic_send_eoi().
     *     No slave/master distinction exists here; every IRQ this
     *     kernel currently routes through IO-APIC still targets the
     *     BSP's own LAPIC specifically (see
     *     ioapic_program_isa_redirects()'s own comment), so this is
     *     correct regardless of which CPU happens to be running this
     *     handler today. */
    if (rust_ioapic_is_active()) {
        rust_apic_send_eoi();
    } else if (irq >= 8) {
        outb(PIC2_COMMAND, PIC_EOI);
        outb(PIC1_COMMAND, PIC_EOI);
    } else {
        outb(PIC1_COMMAND, PIC_EOI);
    }

    if (irq_handlers[irq]) {
        irq_handlers[irq](regs);
    }
}
