; kernel/arch/x86/cpu/ap_trampoline.s - Phase 56: the AP (secondary CPU)
; bootstrap trampoline.
;
; Deliberately NOT named *.asm - the Makefile's ASM_SOURCES glob is
; `find $(KERNEL_DIR) -name "*.asm"`, and every match gets compiled as
; a normal ELF32 object (`nasm -f elf32`) and linked straight into the
; kernel image alongside every other .o file. That is exactly wrong
; for this file: it must be assembled as a flat, position-dependent
; binary blob (`nasm -f bin`, see the Makefile's own dedicated rule
; for AP_TRAMPOLINE_BIN) and copied byte-for-byte to a fixed *physical*
; address at runtime, not linked as part of the kernel's own ELF at
; all. The kernel embeds the resulting .bin directly into
; kernel/rust/apic.rs via Rust's `include_bytes!`, so this file's only
; consumer is that one Makefile rule.
;
; --- Why a trampoline is needed at all ---
; When the BSP (boot CPU) sends an INIT-SIPI-SIPI sequence to wake a
; second CPU (see kernel/rust/apic.rs's own smp_boot_aps()), that CPU
; does not start executing wherever the BSP currently is - it starts
; completely cold, in 16-bit real mode, at a physical address chosen
; by the SIPI's own 8-bit vector (vector << 12), with paging off,
; interrupts off, and no GDT/IDT loaded. Every register/table this
; kernel's C and Rust code assumes already exists (a flat GDT, a
; working IDT, CR0.PG set, a valid stack) has to be built from
; scratch, by hand-written assembly, before it is safe to jump into
; any compiled C or Rust function at all. That bootstrapping code is
; what this file is.
;
; --- Where it lives, and why ---
; ORG 0x8000 (32KB into physical memory): comfortably below the 1MB
; mark the real kernel itself loads at (see tools/linker.ld: `. = 1M`)
; and well above the BIOS data area / real-mode IVT (0x0-0x1000) and
; any bootloader scratch space, so it is genuinely free at the point
; this kernel copies it there. 0x8000 also happens to already fall
; inside paging.c's own static 0-64MB identity map, so the copy itself
; (an ordinary memcpy from Rust) needs no on-demand mapping the way
; kernel/rust/apic.rs's own LAPIC/IO-APIC MMIO access does.
;
; A SIPI's vector field is this address shifted right by 12 (i.e. the
; trampoline's physical address must be exactly page-aligned on a
; 4096-byte boundary, and expressible in one byte after that shift -
; 0x8000 >> 12 = 0x08, comfortably valid). kernel/rust/apic.rs hard-
; codes both 0x8000 and the corresponding vector byte 0x08 - if this
; ORG ever changes, that constant must change with it.
;
; --- The mailbox: how the BSP hands each AP its own identity ---
; Every AP that ever runs this code executes the *exact same bytes* at
; the *exact same physical address* (this kernel brings CPUs up one at
; a time, reusing the same trampoline copy for each - see
; smp_boot_aps()'s own comment for why that sequential design was
; chosen over giving each AP its own trampoline copy). What has to
; differ per-AP (which kernel stack to use, and confirmation that this
; specific AP has actually left the trampoline) lives in a small fixed
; "mailbox" - six 32-bit words at a fixed physical address, 0x7000,
; chosen simply because it is free low memory that doesn't collide
; with the trampoline itself, the BIOS data area, or the real-mode
; IVT. This address is NOT computed from any label in this file - it
; is a hard-coded constant that kernel/rust/apic.rs's own AP_MAILBOX_*
; constants must match byte-for-byte, documented in both places.
;
;   0x7000  ap_stack_top          - ESP for the new AP to run on
;   0x7004  ap_page_directory     - CR3 value (the kernel's own, shared
;                                    across every CPU - see the comment
;                                    at the CR3 load below)
;   0x7008  ap_kernel_gdt_ptr     - address of gdt.c's own gdt_pointer
;                                    struct (already in the exact
;                                    {limit:u16, base:u32} packed
;                                    format LGDT expects)
;   0x700C  ap_kernel_idt_ptr     - same idea, for idt.c's idt_pointer
;   0x7010  ap_entry_point        - address of the Rust/C function to
;                                    jump to once everything above is
;                                    live (kernel/rust/apic.rs's own
;                                    rust_ap_main)
;   0x7014  ap_ack                - the AP writes 1 here as the very
;                                    last thing it does before leaving
;                                    this file's code; the BSP spins on
;                                    this (bounded, not forever - see
;                                    smp_boot_aps()) before it is safe
;                                    to overwrite the mailbox for the
;                                    next CPU.
;
; This kernel's PMM/paging convention elsewhere ("phys == virt for any
; already-mapped kernel memory") makes reading/writing 0x7000 directly
; from 32-bit flat-mode Rust just as safe as reading it here in the
; trampoline's own 32-bit half - no translation needed on either side.
;
; --- The two-stage GDT ---
; This code cannot LGDT the kernel's own real GDT immediately: the
; first far jump into 32-bit mode has to happen before this code can
; safely load a 32-bit CR3 or lidt the real kernel IDT (both of those
; still need to run in genuine 32-bit protected mode), which itself
; needs *some* valid flat code/data descriptor pair to jump through.
; So this file carries its own tiny, throwaway two-descriptor GDT
; (null + flat 32-bit code + flat 32-bit data, byte-for-byte the same
; encoding kernel/arch/x86/cpu/gdt.c's own gdt_init() builds for
; GDT_KERNEL_CODE/GDT_KERNEL_DATA) purely to make that first jump.
; Once paging is live, this code loads the REAL, shared kernel GDT
; (via the mailbox's ap_kernel_gdt_ptr) and reloads every segment
; register a second time - selectors 0x08/0x10 happen to mean the same
; thing in both tables by construction, so the only reason the second
; reload is required at all is that CS's *descriptor cache* was
; populated from the temporary table and has to be refreshed from the
; real one, which only a fresh far jump/reload actually does.

BITS 16
ORG 0x8000

ap_trampoline_start:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax          ; SS/ESP are reloaded for real once in 32-bit
                         ; mode (see below) - this just leaves them in
                         ; a sane, defined state until then.

    lgdt [temp_gdt_descriptor]

    mov eax, cr0
    or eax, 1            ; CR0.PE
    mov cr0, eax

    ; Far jump into the temporary GDT's flat code segment (selector
    ; 0x08). This is the instruction that actually starts 32-bit
    ; execution - everything after it runs as protected-mode code.
    jmp 0x08:pm_entry

BITS 32
pm_entry:
    mov ax, 0x10          ; temporary GDT's flat data segment
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax

    ; This AP's own kernel stack, allocated by the BSP specifically
    ; for it (kernel/rust/apic.rs's own smp_boot_aps(), one call to
    ; pmm_alloc_contiguous() per AP - see that function's comment for
    ; why a contiguous allocation, not one arbitrary frame at a time,
    ; is required for a stack) before this AP was ever sent a SIPI.
    mov esp, [0x7000]     ; ap_stack_top

    ; Enable paging using the *same* CR3 every other CPU (including
    ; the BSP) already uses. This is deliberate, not a shortcut: every
    ; CPU in this kernel shares one identity-mapped view of physical
    ; memory (paging.c's own build_identity_map(), 0-64MB) plus
    ; whatever kernel/rust/apic.rs's on-demand mapping has added for
    ; LAPIC/IO-APIC MMIO - there is currently no such thing as a
    ; genuinely *private* per-CPU address space in this kernel (that
    ; only exists per-*process*, via paging_create_address_space()),
    ; so there is nothing for an AP to have of its own here yet. A
    ; future phase that runs real user processes on more than one CPU
    ; will need to revisit this; today, every CPU seeing the exact
    ; same kernel mapping is both correct and exactly what a kernel-
    ; only idle loop needs.
    mov eax, [0x7004]     ; ap_page_directory
    mov cr3, eax
    mov eax, cr0
    or eax, 0x80000000    ; CR0.PG
    mov cr0, eax

    ; Now that paging is live, this AP can safely reach the kernel's
    ; own real GDT/IDT (ordinary kernel data, identity-mapped) instead
    ; of the throwaway temporary GDT above.
    mov eax, [0x7008]     ; ap_kernel_gdt_ptr -> gdt.c's gdt_pointer
    lgdt [eax]
    mov eax, [0x700C]     ; ap_kernel_idt_ptr -> idt.c's idt_pointer
    lidt [eax]

    ; Reload CS from the *real* kernel GDT - selector 0x08 means "flat
    ; ring-0 code" in both tables by construction (see gdt.c), but the
    ; CPU's CS descriptor cache still has to be explicitly refreshed
    ; from the new table, which only a far jump actually does.
    jmp 0x08:.reload_segments
.reload_segments:
    mov ax, 0x10           ; GDT_KERNEL_DATA, in the real kernel GDT now
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax

    ; Signal the BSP that this AP has finished reading everything it
    ; needs out of the mailbox - from this point on the BSP is free to
    ; overwrite 0x7000-0x7013 for the next CPU it brings up. This must
    ; be the last mailbox interaction on this AP's side, and it must
    ; come after every read above, or the BSP could reuse the mailbox
    ; for CPU N+1 while CPU N was still reading CPU N+1's values.
    mov dword [0x7014], 1  ; ap_ack

    ; Hand off to real kernel code. Never returns.
    mov eax, [0x7010]      ; ap_entry_point -> rust_ap_main
    jmp eax

ALIGN 8
temp_gdt_start:
    dq 0                                ; null descriptor

    ; Flat 32-bit ring-0 code segment (selector 0x08) - byte-for-byte
    ; the same encoding as gdt.c's own
    ; gdt_set_gate(1, 0, 0xFFFFFFFF, 0x9A, 0xCF).
    dw 0xFFFF, 0x0000
    db 0x00, 0x9A, 0xCF, 0x00

    ; Flat 32-bit ring-0 data segment (selector 0x10) - matches
    ; gdt_set_gate(2, 0, 0xFFFFFFFF, 0x92, 0xCF).
    dw 0xFFFF, 0x0000
    db 0x00, 0x92, 0xCF, 0x00
temp_gdt_end:

temp_gdt_descriptor:
    dw temp_gdt_end - temp_gdt_start - 1
    dd temp_gdt_start

ap_trampoline_end:
