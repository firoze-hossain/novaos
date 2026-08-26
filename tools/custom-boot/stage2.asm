; stage2.asm - the real bootloader logic (Phase 28c). Runs in real
; mode first (BIOS calls only work there), then switches to 32-bit
; protected mode partway through, since everything after that point
; (ELF parsing, memory copies to addresses above 1MB) is far simpler
; with flat 32-bit addressing than it would be in real mode.
;
; Sequence: detect memory via BIOS E820h (must happen in real mode)
; -> load the kernel ELF from disk into a low temporary buffer via
; INT 13h extended (LBA) reads (also real-mode only; legacy CHS reads
; are avoided here since they cap out at 63 sectors/track and this
; kernel is far larger than that) -> enable the A20 line -> load a
; flat GDT and enter protected mode -> in protected mode, parse the
; ELF's program headers and copy each PT_LOAD segment to its real
; destination (>= 1MB, unreachable directly in real mode) -> build a
; Multiboot-compatible info structure from the E820 data collected
; earlier -> jump to the kernel's entry point with EAX/EBX set
; exactly as kernel/arch/x86/boot/multiboot.asm's _start expects from
; GRUB, so the exact same kernel binary runs identically either way.

BITS 16
ORG 0x7E00

KERNEL_TEMP_LINEAR equ 0x10000    ; kernel ELF loaded here in real mode
KERNEL_LOAD_LBA equ 35             ; must equal 1 + stage1.asm's
                                     ; STAGE2_SECTOR_COUNT - see that
                                     ; constant's comment for the full
                                     ; manual-sync dependency note
                                    ; + stage2 (LBAs 1-16, 16 sectors)
KERNEL_MAX_SECTORS equ 800        ; 400KB - comfortably above the
                                    ; ~388KB kernel binary this was
                                    ; verified against; a much larger
                                    ; future kernel would need this
                                    ; raised, a real, documented limit
MULTIBOOT_MAGIC equ 0x2BADB002

start:
    mov [boot_drive], dl   ; stage1 explicitly reloads DL from its own
                            ; saved value right before jumping here

    mov si, msg_stage2
    call print_string

    ; --- Memory detection via BIOS INT 15h, EAX=E820h ---
    ; Deliberately convenient: each E820 entry (base:8, length:8,
    ; type:4 = 20 bytes) already matches the multiboot_mmap_entry_t
    ; layout kernel/arch/x86/boot/multiboot.h expects, once a 4-byte
    ; "size" field (always 20 here) is written immediately before it -
    ; this loop builds the Multiboot-format table directly, with no
    ; separate translation step needed.
    mov di, mmap_buffer
    xor ebx, ebx
    mov dword [mmap_entry_count], 0
.e820_loop:
    mov eax, 0xE820
    mov edx, 0x534D4150       ; 'SMAP'
    mov ecx, 20
    add di, 4                  ; leave room for this entry's "size" field
    int 0x15
    jc .e820_done              ; CF set = error or end of list
    cmp eax, 0x534D4150
    jne .e820_done

    mov eax, 20
    mov [di - 4], eax          ; the "size" field multiboot expects
    add di, 20                 ; advance past this 20-byte entry
    inc dword [mmap_entry_count]

    cmp ebx, 0
    je .e820_done               ; EBX=0 means that was the last entry
    cmp dword [mmap_entry_count], 64
    jae .e820_done               ; a generous, fixed cap on entries
    jmp .e820_loop
.e820_done:
    mov si, msg_e820_done
    call print_string

    ; --- Load the kernel ELF via INT 13h AH=42h (extended/LBA read) -
    ; simpler and correct for a read this large, unlike legacy CHS
    ; (AH=02h), which caps out at 63 sectors/track and doesn't support
    ; a plain linear sector count the way LBA does. ---
    mov si, msg_loading_kernel
    call print_string

    mov word [dap_offset], 0
    mov word [dap_segment], 0x1000    ; KERNEL_TEMP_LINEAR (0x10000) as
                                        ; segment 0x1000 : offset 0
    mov dword [dap_lba_low], KERNEL_LOAD_LBA
    mov dword [dap_lba_high], 0
    mov word [sectors_remaining], KERNEL_MAX_SECTORS
.load_loop:
    cmp word [sectors_remaining], 0
    je .load_done

    mov ax, [sectors_remaining]
    cmp ax, 64
    jbe .chunk_ok
    mov ax, 64
.chunk_ok:
    mov [dap_count], ax

    mov si, dap
    mov ah, 0x42
    mov dl, [boot_drive]
    int 0x13
    jc .disk_error

    mov ax, [dap_count]
    shl ax, 5                          ; sectors * 512 / 16 = sectors*32
    add [dap_segment], ax

    movzx eax, word [dap_count]
    add [dap_lba_low], eax

    mov ax, [dap_count]
    sub [sectors_remaining], ax
    jmp .load_loop
.load_done:
    mov si, msg_kernel_loaded
    call print_string
    jmp enable_a20

.disk_error:
    mov si, msg_disk_error
    call print_string
.hang_real:
    hlt
    jmp .hang_real

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

; --- A20 line enable (the "fast A20" method via port 0x92) ---
enable_a20:
    in al, 0x92
    or al, 2
    out 0x92, al

    ; --- Enter protected mode ---
    cli
    lgdt [gdt_descriptor]
    mov eax, cr0
    or eax, 1
    mov cr0, eax
    jmp CODE_SEG:protected_mode_entry

BITS 32
protected_mode_entry:
    mov ax, DATA_SEG
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax
    mov esp, 0x90000

    ; --- Parse the ELF header at the temp load buffer and copy each
    ; PT_LOAD segment to its real destination - the same structural
    ; fields kernel/task/elf.c's own loader reads (e_entry, e_phoff,
    ; e_phnum, and each Elf32_Phdr's p_type/p_offset/p_vaddr/p_filesz/
    ; p_memsz), applied here to load the kernel itself rather than a
    ; userland program. ---
    mov esi, KERNEL_TEMP_LINEAR
    mov eax, [esi + 24]        ; e_entry
    mov [kernel_entry], eax
    mov eax, [esi + 28]        ; e_phoff
    add eax, esi
    mov ebp, eax                ; ebp = pointer to current program header
    movzx ecx, word [esi + 44]  ; e_phnum

.ph_loop:
    cmp ecx, 0
    je .ph_done
    mov eax, [ebp + 0]          ; p_type
    cmp eax, 1                  ; PT_LOAD
    jne .ph_next

    push ecx                     ; save phnum-remaining across the copy
    push esi                     ; save the ELF base pointer

    mov eax, [ebp + 4]           ; p_offset
    add eax, esi                 ; source = temp buffer + p_offset
    mov edi, [ebp + 8]           ; p_vaddr = destination (identity-
                                   ; mapped at this stage, so this is
                                   ; also the physical address)
    mov ebx, [ebp + 16]          ; p_filesz
    mov esi, eax
    mov ecx, ebx
    cld
    rep movsb                     ; copy p_filesz bytes

    mov eax, [ebp + 20]          ; p_memsz
    sub eax, ebx                  ; bytes needing zero-fill
    jz .zero_done
    mov ecx, eax
    xor al, al
    rep stosb                     ; edi is already positioned right
                                    ; after the copied bytes
.zero_done:
    pop esi
    pop ecx

.ph_next:
    add ebp, 32                    ; sizeof(Elf32_Phdr)
    dec ecx
    jmp .ph_loop
.ph_done:

    ; --- Build the multiboot_info_t structure the kernel expects,
    ; using the E820 data collected in real mode earlier. ---
    mov dword [mb_info + 0], 0x00000040    ; flags: MULTIBOOT_INFO_MEM_MAP
    mov eax, [mmap_entry_count]
    imul eax, eax, 24
    mov [mb_info + 44], eax                ; mmap_length
    mov dword [mb_info + 48], mmap_buffer  ; mmap_addr

    ; --- Jump to the kernel exactly as GRUB would ---
    mov eax, MULTIBOOT_MAGIC
    mov ebx, mb_info
    jmp [kernel_entry]

BITS 16
; --- GDT: flat 4GB code and data segments, ring 0 ---
gdt_start:
    dq 0x0000000000000000       ; null descriptor
gdt_code:
    dw 0xFFFF, 0x0000
    db 0x00, 10011010b, 11001111b, 0x00
gdt_data:
    dw 0xFFFF, 0x0000
    db 0x00, 10010010b, 11001111b, 0x00
gdt_end:

gdt_descriptor:
    dw gdt_end - gdt_start - 1
    dd gdt_start

CODE_SEG equ gdt_code - gdt_start
DATA_SEG equ gdt_data - gdt_start

boot_drive: db 0
mmap_entry_count: dd 0
kernel_entry: dd 0

; Disk Address Packet for INT 13h AH=42h extended reads.
dap:
    db 16                  ; size of this packet
    db 0                    ; reserved
dap_count: dw 0             ; sectors to transfer this call
dap_offset: dw 0            ; transfer buffer offset
dap_segment: dw 0           ; transfer buffer segment
dap_lba_low: dd 0           ; starting LBA (low 32 bits)
dap_lba_high: dd 0          ; starting LBA (high 32 bits, always 0 here)

sectors_remaining: dw 0

msg_stage2: db "NovaOS stage2: detecting memory...", 13, 10, 0
msg_e820_done: db "Memory map OK. Loading kernel...", 13, 10, 0
msg_loading_kernel: db "Reading kernel from disk...", 13, 10, 0
msg_kernel_loaded: db "Kernel loaded. Entering protected mode...", 13, 10, 0
msg_disk_error: db "Kernel disk read error!", 13, 10, 0

mb_info:
    times 64 db 0              ; multiboot_info_t - only the fields
                                ; this bootloader fills in (flags,
                                ; mmap_length, mmap_addr) are ever
                                ; written; the rest stay zero, matching
                                ; "not filled in" per the flags word

times 17408 - ($ - $$) db 0   ; pad stage2 (code + mb_info together)
                                ; to exactly 34 sectors - must match
                                ; stage1.asm's STAGE2_SECTOR_COUNT (see
                                ; its comment). mb_info must come
                                ; BEFORE this padding, not after, or
                                ; its 64 bytes silently add on top of
                                ; the intended total instead of being
                                ; included within it (a real bug this
                                ; exact ordering fixes - see git log).

mmap_buffer:
