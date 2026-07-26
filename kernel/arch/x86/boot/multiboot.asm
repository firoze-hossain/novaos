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
    ; GRUB hands off with EAX = multiboot magic (0x2BADB002) and
    ; EBX = physical address of the multiboot_info_t structure.
    ; Neither register is touched by setting up our own stack, but we
    ; save them to registers that survive it anyway for clarity, then
    ; push them as kernel_main(uint32_t magic, uint32_t mbi_addr)'s
    ; arguments (cdecl: pushed right-to-left).
    mov edi, eax            ; edi = multiboot magic
    mov esi, ebx            ; esi = multiboot info pointer

    mov esp, stack_top

    push esi                ; 2nd arg: mbi_addr
    push edi                ; 1st arg: magic
    call kernel_main

    ; Halt if kernel returns
    cli
.hang:
    hlt
    jmp .hang

section .bss
align 16
stack_bottom:
    resb 65536              ; 64KB stack
stack_top:
