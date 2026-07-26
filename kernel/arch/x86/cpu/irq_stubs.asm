; irq_stubs.asm - low-level entry points for hardware IRQs (vectors 32-47)
;
; Structurally identical to isr_stubs.asm: hardware interrupts never carry
; a CPU-pushed error code, so we always push a dummy 0.

section .text
extern irq_handler

%macro IRQ_STUB 2
global irq%1
irq%1:
    cli
    push dword 0     ; dummy error code
    push dword %2     ; interrupt number (32 + IRQ line)
    jmp irq_common_stub
%endmacro

IRQ_STUB 0,  32
IRQ_STUB 1,  33
IRQ_STUB 2,  34
IRQ_STUB 3,  35
IRQ_STUB 4,  36
IRQ_STUB 5,  37
IRQ_STUB 6,  38
IRQ_STUB 7,  39
IRQ_STUB 8,  40
IRQ_STUB 9,  41
IRQ_STUB 10, 42
IRQ_STUB 11, 43
IRQ_STUB 12, 44
IRQ_STUB 13, 45
IRQ_STUB 14, 46
IRQ_STUB 15, 47

irq_common_stub:
    pusha

    mov ax, ds
    push eax

    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    push esp
    call irq_handler
    add esp, 4

    pop eax
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    popa
    add esp, 8
    sti
    iret
