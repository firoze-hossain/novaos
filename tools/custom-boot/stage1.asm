; stage1.asm - the 512-byte MBR boot sector for NovaOS's custom
; bootloader (Phase 28c). This is deliberately kept as simple as
; possible: its only job is reading Stage 2 (a larger, more capable
; real-mode program - see stage2.asm) from the sectors immediately
; following this one into memory, then jumping to it. Everything
; genuinely interesting (memory detection, protected mode, loading
; the actual kernel) happens in Stage 2, which has far more room to
; work with than this sector's 512-byte budget.
;
; This is a purely additive, parallel boot path - novaos.iso (GRUB)
; and disk.img (the partitioned FAT32+ext2 data disk) are completely
; untouched by this. See tools/build-custom-boot-image.sh for how
; this assembles into its own, separate disk image, and PROGRESS.md
; for the full scope note.

BITS 16
ORG 0x7C00

start:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7C00      ; stack grows down from right below us
    sti

    mov [boot_drive], dl ; BIOS passes the boot drive number in DL -
                          ; saved so stage2 can keep reading from the
                          ; same physical drive rather than assuming one

    mov si, msg_loading
    call print_string

    ; Load STAGE2_SECTOR_COUNT sectors starting right after this boot
    ; sector (LBA 1) into memory at 0x0000:0x7E00 - immediately
    ; following this sector's own 512 bytes, a common, simple
    ; convention that avoids needing a second segment.
    mov ah, 0x02        ; BIOS: read sectors (CHS)
    mov al, STAGE2_SECTOR_COUNT
    mov ch, 0           ; cylinder 0
    mov cl, 2           ; sector 2 (1-indexed; sector 1 is this boot sector)
    mov dh, 0           ; head 0
    mov dl, [boot_drive]
    mov bx, 0x7E00
    int 0x13
    jc disk_error

    mov si, msg_ok
    call print_string

    mov dl, [boot_drive]   ; reload explicitly rather than assume DL
                            ; survived the INT 13h call above unchanged
    jmp 0x0000:0x7E00      ; hand off to stage2 (DL = boot drive)

disk_error:
    mov si, msg_error
    call print_string
.hang:
    hlt
    jmp .hang

; Prints a null-terminated string via BIOS teletype output (INT 10h,
; AH=0Eh) AND directly to COM1 (port 0x3F8) - teletype for anyone
; watching the actual screen, serial for this project's established
; headless (-display none -serial file:...) testing methodology,
; which every other phase has relied on and this one needs to match.
print_string:
    lodsb
    or al, al
    jz .done
    push ax
    mov ah, 0x0E
    mov bh, 0
    int 0x10
    pop ax
    call serial_putc
    jmp print_string
.done:
    ret

; al = character to send. Polls the line status register (bit 5 =
; transmit holding register empty) before writing - correct even
; though QEMU's simple serial emulation would likely also work
; without it at this low a data rate.
serial_putc:
    push dx
    push ax
.wait:
    mov dx, 0x3FD
    in al, dx
    test al, 0x20
    jz .wait
    pop ax
    mov dx, 0x3F8
    out dx, al
    pop dx
    ret

boot_drive: db 0
msg_loading: db "NovaOS stage1: loading stage2...", 13, 10, 0
msg_ok: db "OK", 13, 10, 0
msg_error: db "Disk read error!", 13, 10, 0

STAGE2_SECTOR_COUNT equ 34 ; 17KB - stage2.asm's own padding (see its
                           ; "times" directive near the end) must
                           ; target the exact same total, and
                           ; stage2's KERNEL_LOAD_LBA constant must
                           ; equal 1 + this value - these three
                           ; numbers are a manually-synchronized
                           ; dependency across two separately-
                           ; assembled files, not something the build
                           ; enforces automatically. Chosen with
                           ; generous headroom (stage2 actually
                           ; assembles to ~16.1KB right now, including
                           ; the mb_info scratch structure past its
                           ; own code) specifically to make that
                           ; manual sync low-risk unless stage2 grows
                           ; substantially. build-image.sh verifies
                           ; this budget isn't exceeded at build time
                           ; rather than trusting it silently.

times 510 - ($ - $$) db 0
dw 0xAA55
