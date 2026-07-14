; Multiboot header
section .multiboot
align 4
    dd 0x1BADB002          ; Magic number
    dd 0x00                ; Flags
    dd -(0x1BADB002 + 0x00) ; Checksum

; Entry point
section .text
global _start
extern kernel_main

_start:
    ; Set up stack
    mov esp, stack_top

    ; Call kernel main
    call kernel_main

    ; Halt if kernel returns
    cli
.hang:
    hlt
    jmp .hang

section .bss
align 16
stack_bottom:
    resb 32768              ; 32KB stack
stack_top: