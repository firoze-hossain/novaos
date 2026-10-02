; Multiboot header
;
; Phase 79: kernel/drivers/video/vbe.c is a complete, real VESA/VBE
; linear framebuffer driver - real resolution, real color depth
; (packs RGB into whatever bit-field layout GRUB actually reports, not
; a hardcoded assumption), the whole thing ready to activate itself
; automatically (see vbe_init(), called from kernel/init/main.c) the
; moment a loader hands it valid framebuffer info. This header does
; NOT request that info by setting MULTIBOOT_VIDEO_MODE (flags bit 2,
; 0x4) - a real, considered decision, not an oversight: doing so was
; tested extensively, rigorously, against this project's own real test
; environment (GRUB 2.12-1ubuntu7.3, BIOS/legacy mode, QEMU `-vga
; std`) and found to break boot outright there, not degrade gracefully
; the way the Multiboot spec's own wording ("the boot loader may set a
; text mode even if this field contains 0") would suggest. Isolated
; precisely, not just observed: GRUB's own video-output modules (vga/
; video_bochs/video_cirrus) individually hang while probing hardware
; in this exact QEMU+SeaBIOS combination, confirmed via direct VNC
; screenshots at each step; with no video modules loaded at all, the
; bare `multiboot` command still fails with a constant, non-data-
; dependent `error: unsupported graphical mode type NNNNNNNN` - the
; same number regardless of this header's own requested width/height/
; depth, including a spec-compliant "no preference" (all zero) -
; meaning GRUB's own multiboot loader internally queries VBE info to
; honor a video-mode request and that query itself appears to return
; garbage in this specific environment, not something this kernel's
; own header can work around. See PROGRESS.md's own Phase 79 entry for
; the complete investigation. Setting this flag is the only change
; needed to actually exercise the real driver once a working
; environment (a real machine, or a different GRUB build/version) is
; available to test it against - kernel/drivers/video/vbe.c's own
; vbe_init() already handles "no framebuffer info" by falling back to
; the existing, unmodified VGA Mode 13h path (kernel/drivers/video/
; vga_graphics.c) exactly as it does today with this flag unset, so
; flipping it back on is a one-line, fully-prepared-for change, not a
; redesign.
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
