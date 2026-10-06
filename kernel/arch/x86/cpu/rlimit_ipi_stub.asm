; Phase 86: the interrupt stub for the per-CPU resource-limit tick.
;
; The second CPU has no timer interrupt of its own, so a process running there
; is never charged, throttled or stopped by the timer tick (which the BSP
; alone receives). While a process with a CPU limit is running on another
; CPU, the BSP's tick sends THIS vector to it (an IPI), and rlimit_ipi_handler
; (kernel/task/rlimit.c) does there what the BSP's tick does for the BSP.
;
; irq_common_stub (irq_stubs.asm) cannot be reused: its C handler indexes a
; 16-entry table by (vector - 32), and this vector is far outside it. The
; frame is built exactly the same way, so the handler receives the same
; registers_t.

section .text
extern rlimit_ipi_handler

global rlimit_ipi_stub
rlimit_ipi_stub:
    cli
    push dword 0        ; dummy error code
    push dword 0xF1     ; the vector (RLIMIT_IPI_VECTOR)
    pusha

    mov ax, ds
    push eax

    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    push esp
    call rlimit_ipi_handler
    add esp, 4

    pop eax
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    popa
    add esp, 8
    iret
