; spurious_stub.asm - Phase 56: handler for the Local APIC's own
; spurious-interrupt vector (0xFF - see kernel/rust/apic.rs's own
; SPURIOUS_VECTOR comment for why that specific vector number, and why
; a real IDT gate has to exist here before any Local APIC is ever
; enabled).
;
; A spurious interrupt, by definition (Intel SDM Vol 3A, 10.9), is one
; the Local APIC itself decided not to actually deliver (most commonly:
; the interrupt was withdrawn by the source in the small window
; between the APIC starting to service it and the CPU actually
; fetching this handler) - there is nothing to acknowledge and, per
; the same SDM section, EOI must NOT be sent for it (unlike every real
; IRQ this kernel's irq.c handles). The correct handler is exactly
; this: do nothing, return.
;
; This is a normal, unlike-the-CPU-exception-stubs interrupt gate: no
; error code (real or dummy) is pushed, and nothing here calls back
; into a shared C dispatch function the way isr_stubs.asm/irq_stubs.asm
; both do for vectors 0-47 - there is exactly one possible cause for
; this specific vector ever firing, so there is nothing for a shared
; dispatcher to route.

section .text
global spurious_interrupt_stub

spurious_interrupt_stub:
    iretd
