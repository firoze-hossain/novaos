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

; Phase 27 (fork): a fork()'d child's kernel stack is built to make
; switch_context() "return" straight to this exact label - the tail
; end of ordinary syscall handling, which restores every register from
; the registers_t sitting on the stack (a full copy of the parent's,
; at the moment of the fork() call, with eax overwritten to 0) and
; irets back to userspace. This is what makes the child "wake up"
; already past the fork() syscall, indistinguishable from the parent
; having returned from the same call - see process_fork() in
; kernel/task/process.c for the stack construction this label's
; address is used in.
global syscall_return_point
syscall_return_point:
    pop eax
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    popa
    add esp, 8
    sti
    iret
