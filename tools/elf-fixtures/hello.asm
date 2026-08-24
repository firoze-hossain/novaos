; hello.asm - a real, independently-compiled ELF32 executable for
; testing NovaOS's ELF loader (Phase 23). Written directly in
; assembly rather than C specifically to avoid any dependency on a C
; runtime/crt0 startup convention this kernel doesn't provide yet -
; full control over exactly what code runs at process entry, the same
; reasoning this whole kernel project has used NASM for throughout.
;
; NovaOS syscall convention (see kernel/arch/x86/cpu/syscall.h):
; int 0x80, EAX = syscall number, EBX/ECX/EDX = up to three arguments.
; SYS_WRITE=1 (EBX = NUL-terminated string), SYS_EXIT=2 (EBX = exit
; code). This is NovaOS's own convention, NOT Linux's - do not confuse
; with real Linux syscall numbers if referencing this file later.
;
; Process entry stack convention (see process_exec() in
; kernel/task/process.c): [esp]=argc, [esp+4]=argv[0], [esp+8]=argv[1],
; ..., [esp+4+4*argc]=NULL, one more NULL (empty envp), then the
; string data - the same raw layout a real C runtime's _start expects.
;
; Build (see tools/elf-fixtures/build.sh):
;   nasm -f elf32 hello.asm -o hello.o
;   ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static -o hello.elf hello.o

BITS 32

SECTION .text
global _start

_start:
    mov ebp, esp
    mov esi, [ebp]         ; esi = argc

    mov eax, 1              ; SYS_WRITE
    mov ebx, msg_welcome
    int 0x80

    cmp esi, 1
    jl .after_argv0
    mov eax, 1
    mov ebx, [ebp+4]        ; argv[0]
    int 0x80
    mov eax, 1
    mov ebx, newline
    int 0x80
.after_argv0:

    cmp esi, 2
    jl .after_argv1
    mov eax, 1
    mov ebx, [ebp+8]        ; argv[1]
    int 0x80
    mov eax, 1
    mov ebx, newline
    int 0x80
.after_argv1:

    mov eax, 2               ; SYS_EXIT
    mov ebx, 42               ; a specific, recognizable exit code
    int 0x80

.hang:                        ; never reached - SYS_EXIT never returns
    jmp .hang

SECTION .data
msg_welcome: db "Hello from a real ELF executable loaded by NovaOS!", 10, 0
newline: db 10, 0
