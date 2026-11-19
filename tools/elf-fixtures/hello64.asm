; hello64.asm - a real, independently-compiled ELF64 executable for
; validating NovaOS's 64-bit ELF loader and x86_64 Long Mode support.
;
; NovaOS x86_64 ABI & Syscall:
; fast syscall / sysret via MSR_LSTAR / POSIX x86_64 translation layer
;
; Build (see tools/elf-fixtures/build.sh):
;   nasm -f elf64 hello64.asm -o hello64.o
;   ld -m elf_x86_64 -Ttext=0x00400000 --entry=_start -static -o hello64.elf hello64.o

BITS 64

SECTION .text
global _start

_start:
    ; SYS_WRITE (1), fd=1, buffer=msg_welcome, count=msg_len
    mov rax, 1
    mov rdi, 1
    lea rsi, [rel msg_welcome]
    mov rdx, msg_len
    syscall

    ; SYS_EXIT (60), exit_code=0
    mov rax, 60
    mov rdi, 0
    syscall

.hang:
    jmp .hang

SECTION .data
msg_welcome: db "Hello from a real 64-bit x86_64 ELF executable on NovaOS!", 10
msg_len: equ $ - msg_welcome
