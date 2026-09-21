; context_switch.asm - the two pieces of hand-written asm that make
; multitasking possible: switch_context() and enter_usermode.
;
; ============================================================
; void switch_context(uint32_t* old_esp_out, uint32_t new_esp)
; ============================================================
; Saves the callee-saved registers and EFLAGS of the CURRENTLY running
; task onto its own stack, records where that left ESP into
; *old_esp_out, then loads ESP = new_esp and restores the same set of
; registers for whatever task that stack belongs to, and `ret`s into
; it.
;
; The "ret" is the whole trick: it doesn't return to whoever called
; switch_context this time - it returns to whatever address is sitting
; at the new stack's current top, which is either:
;   (a) a real return address left there the last time THIS OTHER task
;       called switch_context itself (the common case: resuming a task
;       that was previously preempted mid-interrupt just unwinds back
;       through its own suspended C call stack, all the way out through
;       irq_common_stub's `iret`), or
;   (b) a fake one process_create_*() wrote there for a task that has
;       never run yet - kernel_task_trampoline or enter_usermode below.
;
; Only EBP/EBX/ESI/EDI + EFLAGS need saving here (not EAX/ECX/EDX):
; this is a normal cdecl function call as far as the C compiler on the
; calling side is concerned, so it has already spilled any caller-saved
; registers it still needed before making the call, exactly as it would
; for any other function call that might clobber them.

section .text
global switch_context

switch_context:
    push ebp
    push ebx
    push esi
    push edi
    pushfd

    mov eax, [esp + 24]     ; old_esp_out (5 pushes above + return addr = 24)
    mov [eax], esp

    mov eax, [esp + 28]     ; new_esp
    mov esp, eax

    popfd
    pop edi
    pop esi
    pop ebx
    pop ebp
    ret

; ============================================================
; enter_usermode - first-run trampoline for a brand new user task
; ============================================================
; process_create_user_task() builds a fake stack frame whose "return
; address" (the slot switch_context's `ret` above jumps to) is this
; function, immediately followed by a 5-dword IRET frame:
; [EIP][CS][EFLAGS][ESP][SS]. IRET only restores CS/EIP/EFLAGS/SS/ESP,
; not the other segment registers, so DS/ES/FS/GS still hold whatever
; the last interrupt prologue left in them (the kernel data selector) -
; loading those with the ring-3 data selector first is required, or
; the very first memory access after `iret` (even implicitly, e.g. for
; a stack push) takes a #GP fault.

global enter_usermode
enter_usermode:
    ; A real, confirmed bug: switch_context()'s own popfd (just before
    ; its `ret` that lands here) already restored THIS task's own
    ; saved eflags - for a brand new task like this one, that's the
    ; 0x202 process_create_user_task_common()/process_exec_internal()/
    ; process_fork() baked in, with IF=1 - meaning interrupts are
    ; already enabled for the entire multi-instruction stretch below,
    ; not just at the final `iret` the way syscall_return_point/
    ; irq_common_stub/isr_common_stub (kernel/arch/x86/cpu/
    ; syscall_stub.asm, irq_stubs.asm, isr_stubs.asm - see their own,
    ; identical fix) already ensure for every *other* return-to-
    ; userspace path. A timer tick firing in this exact window -
    ; before this function's own segment-register setup and `iret`
    ; complete - is a real, unguarded hazard for exactly this reason,
    ; on a task that (being brand new) has no prior suspended call
    ; chain to safely unwind back through if preempted here. `cli`
    ; closes the same gap the other three paths already close, by the
    ; same means: `iret` below still correctly re-enables interrupts
    ; on its own, since it restores this task's real, intended eflags
    ; (IF=1) from the fake stack frame process_create_*() built.
    cli
    mov ax, 0x23            ; GDT_USER_DATA (0x20 | ring 3)
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    iret
