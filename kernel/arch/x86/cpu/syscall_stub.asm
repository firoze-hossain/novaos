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

    ; A real, confirmed bug used to have `sti` here, right before
    ; `iret`. It was entirely redundant - `iret` itself restores
    ; EFLAGS (including IF) from the stack it's about to pop, and the
    ; original user-mode EFLAGS pushed when this syscall's own `int
    ; 0x80` first fired already had IF=1 (user-mode code always runs
    ; with interrupts enabled) - so `iret` alone already re-enables
    ; interrupts correctly, at the exact right instant. The explicit
    ; `sti` did something actively harmful instead: it re-enabled
    ; interrupts one full instruction *before* `iret` consumed the
    ; real [EIP,CS,EFLAGS,ESP,SS] return frame still sitting on the
    ; stack - a real, unguarded window where a timer tick (or any
    ; other interrupt) could fire, preempt this exact process via
    ; do_schedule()/switch_context() mid-return, and - depending on
    ; exactly what ran before this process was later resumed - leave
    ; that still-pending return frame corrupted by the time this `iret`
    ; finally executed. Confirmed as the real, root explanation for a
    ; long-running, non-deterministic corruption investigation: the
    ; symptom was always execution jumping to a wholly invalid
    ; address immediately after a syscall returned, reproducible even
    ; on a single CPU core (ruling out any multi-core race), and this
    ; exact `sti`-before-`iret` gap is the one place in this file
    ; where an interrupt firing at precisely the wrong instant can
    ; corrupt an otherwise-intact return path. The identical pattern
    ; was found and fixed the same way in kernel/arch/x86/cpu/
    ; irq_stubs.asm and isr_stubs.asm - not just here, since IRQ and
    ; exception handlers share the exact same hazard on their own
    ; return paths.
    iret
