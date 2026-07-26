; syscall_stub.asm - int 0x80 entry point
;
; Structurally identical to an ISR stub (same registers_t layout, same
; segment-swap dance), kept separate from isr_stubs.asm rather than
; reusing isr_common_stub because isr.c's exception_handlers[] array is
; fixed at 32 entries (vectors 0-31) - vector 0x80 (128) would index
; straight past the end of it.

section .text
extern syscall_handler

global isr128
isr128:
    cli
    push dword 0     ; dummy error code (INT has none)
    push dword 128   ; vector number, for consistency with registers_t
    jmp syscall_common_stub

syscall_common_stub:
    pusha

    mov ax, ds
    push eax

    mov ax, 0x10     ; GDT_KERNEL_DATA
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    push esp
    call syscall_handler
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
