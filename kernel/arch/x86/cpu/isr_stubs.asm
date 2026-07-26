; isr_stubs.asm - low-level entry points for CPU exceptions (vectors 0-31)
;
; The CPU only pushes an error code automatically for a handful of
; exceptions (8, 10-14, 17). For the rest we push a dummy 0 ourselves so
; that isr_common_stub can build a uniform registers_t regardless of
; which vector fired.

section .text
extern isr_handler

global idt_flush
idt_flush:
    mov eax, [esp + 4]
    lidt [eax]
    ret

%macro ISR_NOERRCODE 1
global isr%1
isr%1:
    cli
    push dword 0        ; dummy error code
    push dword %1        ; interrupt number
    jmp isr_common_stub
%endmacro

%macro ISR_ERRCODE 1
global isr%1
isr%1:
    cli
    push dword %1        ; interrupt number (CPU already pushed err code)
    jmp isr_common_stub
%endmacro

ISR_NOERRCODE 0
ISR_NOERRCODE 1
ISR_NOERRCODE 2
ISR_NOERRCODE 3
ISR_NOERRCODE 4
ISR_NOERRCODE 5
ISR_NOERRCODE 6
ISR_NOERRCODE 7
ISR_ERRCODE   8
ISR_NOERRCODE 9
ISR_ERRCODE   10
ISR_ERRCODE   11
ISR_ERRCODE   12
ISR_ERRCODE   13
ISR_ERRCODE   14
ISR_NOERRCODE 15
ISR_NOERRCODE 16
ISR_ERRCODE   17
ISR_NOERRCODE 18
ISR_NOERRCODE 19
ISR_NOERRCODE 20
ISR_NOERRCODE 21
ISR_NOERRCODE 22
ISR_NOERRCODE 23
ISR_NOERRCODE 24
ISR_NOERRCODE 25
ISR_NOERRCODE 26
ISR_NOERRCODE 27
ISR_NOERRCODE 28
ISR_NOERRCODE 29
ISR_NOERRCODE 30
ISR_NOERRCODE 31

isr_common_stub:
    pusha                ; edi, esi, ebp, esp, ebx, edx, ecx, eax

    mov ax, ds
    push eax             ; save the data segment

    mov ax, 0x10         ; GDT_KERNEL_DATA - drivers must not run on a
    mov ds, ax           ; leftover user/unknown segment while in the
    mov es, ax           ; kernel, regardless of what was interrupted.
    mov fs, ax
    mov gs, ax

    push esp             ; pass registers_t* to the C handler
    call isr_handler
    add esp, 4

    pop eax              ; restore the original data segment
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    popa
    add esp, 8           ; drop int_no and err_code
    sti
    iret
