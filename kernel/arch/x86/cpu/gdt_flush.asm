; gdt_flush.asm - load GDTR and reload every segment register
;
; The CPU only reads the GDT when a segment register is *loaded*, so after
; LGDT we still have to reload CS (via a far jump - "far" meaning it
; also changes the code segment selector) and the data segments.

section .text
global gdt_flush

gdt_flush:
    mov eax, [esp + 4]     ; first argument: pointer to gdt_ptr struct
    lgdt [eax]

    mov ax, 0x10            ; GDT_KERNEL_DATA selector
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax

    jmp 0x08:.flush         ; GDT_KERNEL_CODE selector, far jump reloads CS
.flush:
    ret
