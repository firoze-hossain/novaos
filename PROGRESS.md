# NovaOS - Progress

This file tracks what is *actually implemented and boot-tested*, as
opposed to PROJECT_PLAN.md, which tracks what's *intended*. Update this
in the same PR as the code it describes.

## Status at a glance

| Phase | Status |
|---|---|
| P1 - Bootloader & Kernel Foundation | Complete |
| P2 - Memory Management & Interrupts | Complete |
| P3 - Filesystem & Drivers | Complete (scoped - see below) |
| P4-P9 | Not started |

## Phase 1 - Bootloader & Kernel Foundation

Unchanged from the original plan: Multiboot-compliant kernel, VGA text
driver with color support, a minimal `printf`-family (`vsnprintf`),
bootable ISO via GRUB. Verified working via `make test` and manual
`make run`.

## Phase 2 - Memory Management & Interrupts

**Status: Complete.** Boot-verified via `make test` (see the CI badge /
`.github/workflows/ci.yml`) and manually in QEMU.

### What was built

- **`kernel/arch/x86/io.h`** - shared `inb`/`outb`/`inw`/`outw`/`io_wait`,
  replacing the driver-local copies that were starting to appear.
- **`kernel/drivers/serial/`** - COM1 16550 UART driver. `kernel_log()`
  now has a real implementation and writes here; this is the primary
  debugging channel (`make debug`, CI's `make test` assertions).
- **`kernel/arch/x86/cpu/gdt.*`** - flat-memory-model GDT with ring 0
  and ring 3 code/data segments. Ring 3 isn't used by anything yet
  (that's Phase 4's usermode work) but the selectors are defined now so
  later code doesn't need to touch this file again.
- **`kernel/arch/x86/cpu/{idt,isr,irq}.*` + `isr_stubs.asm` +
  `irq_stubs.asm`** - full 256-entry IDT; vectors 0-31 are CPU
  exceptions (all 32 are wired up and will `kernel_panic()` with the
  exception name, vector, error code, and faulting EIP if unhandled);
  vectors 32-47 are hardware IRQs after a PIC remap. Every IRQ line is
  masked until a driver explicitly calls `register_irq_handler()`.
- **`kernel/drivers/timer/`** - PIT driver on IRQ0, configurable
  frequency (100 Hz by default), tick counter, `timer_sleep_ms()`.
- **`kernel/drivers/keyboard/`** - PS/2 driver on IRQ1, Scan Code Set 1
  to ASCII (US QWERTY), shift-key tracking, a 256-byte ring buffer, and
  a blocking `keyboard_get_char()`.
- **`kernel/arch/x86/mm/heap.*`** - first-fit free-list `kmalloc`/`kfree`
  over a static 2 MB arena, with per-block magic numbers so a corrupted
  or double-freed block causes an immediate, diagnosable panic instead
  of silent heap corruption.
- **`kernel/shell/`** - minimal interactive shell (`help`, `about`,
  `echo`, `clear`, `meminfo`, `uptime`, `reboot`) that exercises the
  entire Phase 2 input pipeline end to end (IDT -> IRQ1 -> keyboard
  buffer -> shell -> VGA).
- **`kernel/init/main.c`** - rewritten around a `kernel_early_init()` /
  `kernel_late_init()` split: everything that must happen before
  interrupts are safe (GDT, IDT/IRQ, heap) happens in early init;
  everything that registers an IRQ handler (timer, keyboard) happens in
  late init, immediately followed by `sti`.

### Verified behavior (this update)

- Clean build with `-Wall -Wextra` on GCC 13 (Ubuntu 24.04): **zero
  warnings**.
- Full boot sequence confirmed via serial log:
  ```
  NovaOS booting (kernel v0.1.0)...
  [ OK ] GDT initialized
  [ OK ] IDT/ISR/IRQ initialized, PIC remapped to 0x20-0x2F
  [ OK ] Heap initialized (2 MB arena)
  [ OK ] PIT timer initialized at 100 Hz (IRQ0)
  [ OK ] PS/2 keyboard initialized (IRQ1)
  [ OK ] Interrupts enabled
  ```
- Interactively tested via the QEMU monitor (`sendkey`): typing at the
  `nova>` prompt, running `help`, and running `echo` correctly reach the
  shell and print to the VGA console; the machine stayed up through a
  multi-second session with the timer firing at 100 Hz and multiple
  keyboard IRQs, with no fault or panic logged.
- `make test` (new in this update) automates the serial-log assertion
  above and is wired into CI.

### Known limitations / follow-ups (tracked for Phase 3+)

- **No paging yet.** The heap is a fixed 2 MB static arena rather than
  a physical-memory-map-aware allocator; NovaOS doesn't parse the
  Multiboot memory map yet. Tracked for Phase 3 alongside paging.
- **`string.c` is missing `strncmp`.** `kernel/shell/shell.c` has a
  local `strncmp_local` static function as a stopgap. Move to
  `kernel/lib/string.c` once a second caller needs it.
- **`vsnprintf` has no width/padding/`%f` support.** Fine for the
  current log messages; will need revisiting once error messages want
  aligned columns.
- **Keyboard layout is hardcoded US QWERTY** with no layout-switching
  hook yet.
- **`reboot` works; there is no `shutdown`.** ACPI power-off isn't
  implemented (would need at least basic ACPI table parsing).
- **Single-core only.** No APIC/SMP bring-up; the (remapped) 8259 PIC
  is used rather than the IOAPIC/LAPIC pair a multi-core build would
  need.
- **The shell has no line history or editing beyond backspace.**
  Acceptable for Phase 2's goal (prove the input pipeline works); a
  real line editor is Phase 4 scope.

## Phase 3 - Paging, Physical Memory & Filesystem

**Status: Complete, at a deliberately scoped-down scope.** Boot-verified
via `make test` and manually in QEMU, including with a real ATA disk
attached.

### What was built

- **`kernel/arch/x86/boot/multiboot.h` + `multiboot.asm`** - the boot
  assembly now preserves EAX (Multiboot magic) and EBX (info struct
  pointer) that GRUB hands off, and passes them into `kernel_main()`.
  Previously these were silently discarded.
- **`kernel/arch/x86/mm/pmm.*`** - bitmap physical frame allocator,
  parses the real BIOS memory map GRUB provides (falls back to
  mem_lower/mem_upper, then to "everything reserved," if the map is
  missing or the boot wasn't via a valid Multiboot loader). Reserves
  the first 1MB and the kernel's own image range.
- **`kernel/arch/x86/mm/paging.*`** - identity-maps physical 0-64MB
  with static (compile-time-sized) page tables and enables paging via
  CR0.PG. Registers a page-fault handler (vector 14) that decodes CR2
  and the error code into a human-readable message before panicking.
- **`kernel/drivers/ata/ata.*`** - polling PIO driver, primary bus,
  master device only. Runs IDENTIFY on boot and logs the drive model
  string.
- **`kernel/fs/fat32.*`** - read-only FAT32: boot sector validation,
  root-directory listing, 8.3-name file lookup and read, FAT cluster-
  chain walking.
- **`kernel/fs/vfs.*`** - thin pass-through wrapper (see the file's own
  header comment for why it's not a real multi-filesystem VFS yet).
- **`kernel/shell/shell.c`** - added `ls` and `cat FILE`; `meminfo` now
  also reports physical frame totals via the PMM.
- **`kernel/init/main.c`** - wires PMM -> paging -> heap into
  `kernel_early_init()`; ATA + FAT32 mount into `kernel_late_init()`,
  followed by an automatic self-test (reads `HELLO.TXT` and logs the
  result) that lets `make test` verify the whole ATA -> FAT32 -> VFS
  chain headlessly, the same way Phase 2's boot markers verified
  GDT/IDT/IRQ without a keyboard attached.
- **`tools/fixtures/HELLO.TXT` + `make disk.img`** - a 64MB FAT32 test
  image built with `mtools` (`mformat`/`mcopy`), so creating it needs
  no loop-device mounting or root privileges on any of Windows/WSL2,
  Linux, or macOS. `make run`/`make debug`/`make test` all depend on it
  and attach it automatically.

### Two real bugs found and fixed while building this

Both were caught by actually booting the code, not by review - which
is exactly the point of `make test` and manual verification before
calling a phase "done":

1. **`uint64_t`/`int64_t` were typedef'd to `unsigned long`/`signed
   long`** in `kernel/include/types.h`. Under `-m32` GCC, `long` is 32
   bits, not 64 - so the Multiboot memory map's genuinely 64-bit
   `addr`/`len` fields were being silently truncated, corrupting the
   struct layout for every field after the first. Fixed to `long long`
   (which is 64 bits regardless of `-m32`/`-m64`).
2. **`itoa()`'s loop condition was `while (num > 0 ...)` on a signed
   `int`.** Any hex value with the high bit set (e.g. a page-fault
   address like `0xDEADB000`, or frankly most physical addresses above
   2GB) is negative as a signed int, so the loop never ran and `%x`
   silently printed an empty string. Caught when the page-fault
   handler's own diagnostic output showed `Page fault at 0x` with the
   address missing. Fixed by adding `utoa()` (unsigned) and having
   `itoa()` use it for every base except base-10 negatives, which get
   a proper sign+magnitude conversion instead.

### Verified behavior (this update)

- Clean build, zero warnings under `-Wall -Wextra`.
- Full boot sequence with a disk attached:
  ```
  NovaOS booting (kernel v0.1.0)...
  [ OK ] GDT initialized
  [ OK ] IDT/ISR/IRQ initialized, PIC remapped to 0x20-0x2F
  [ OK ] PMM initialized (131040 frames tracked, 511MB)
  [ OK ] Paging enabled (identity-mapped 0-64MB)
  [ OK ] Heap initialized (2 MB arena)
  [ OK ] PIT timer initialized at 100 Hz (IRQ0)
  [ OK ] PS/2 keyboard initialized (IRQ1)
  [ OK ] ATA primary master detected (LBA28 PIO): QEMU HARDDISK
  [ OK ] FAT32 mounted (cluster=512B, root_cluster=2)
  [ OK ] Interrupts enabled
  [ OK ] FILE READ OK: HELLO.TXT (68 bytes): Hello from NovaOS FAT32! ...
  ```
- Deliberately triggered a page fault (`*(volatile int*)0xDEADB000 = 42`,
  temporarily wired to a hidden shell command, removed before this was
  finalized) and confirmed a clean panic instead of a triple fault:
  ```
  [FAULT] Page fault at 0xDEADB000 (eip=0x100D7F): page not present, write, kernel mode
  [PANIC] Page Fault
  ```
- Interactively verified `ls` and `cat HELLO.TXT` via the QEMU monitor
  (typed keystrokes + screendump) with a disk attached; both worked,
  and the serial log showed no faults during the session, including
  while the timer and keyboard IRQs continued firing during the ATA
  PIO transfer.
- Confirmed boot **without** a disk attached still succeeds cleanly
  (`ATA primary master: not present`, filesystem self-test silently
  skipped, no crash) - a disk is optional, not required, to boot.
- Diagnosed and fixed a QEMU boot-order gotcha along the way: attaching
  a second (non-bootable) `-drive` alongside `-cdrom` can make the BIOS
  try to boot from the data disk instead, which looks exactly like a
  silent hang (no error, no serial output). Fixed with `-boot order=d`,
  now baked into the Makefile's `DISK_FLAGS`.

### Known limitations / follow-ups (tracked for Phase 4+)

- **No per-process address spaces.** Paging is on, but there's a
  single flat identity-mapped 64MB address space shared by everything -
  real virtual memory isolation is Phase 4 scope (needs processes to
  isolate first).
- **No NX bit.** This is 32-bit non-PAE paging; NX requires PAE or long
  mode. Every mapped page is both writable and executable. Tracked as
  future work alongside PAE/paging rework, if it happens.
- **PMM tracks at most the first 1GB of RAM** (a fixed 32KB bitmap,
  sized to avoid needing a heap to size itself with). Fine for this
  kernel's current needs; would need to grow for larger configurations.
- **Identity map is a fixed 64MB**, chosen so paging_init() has no
  dependency on the heap or PMM being ready first. `paging_map()` for
  mapping arbitrary additional pages doesn't exist yet - add when
  something actually needs memory above 64MB mapped.
- **Heap is still the Phase 2 static 2MB arena**, not backed by the
  PMM/paging system. Rewriting it to grow dynamically via
  `pmm_alloc_frame()` is a natural next step but wasn't required for
  Phase 3's filesystem goal, so it was left alone to keep this phase's
  blast radius smaller.
- **ATA driver is PIO-polling, primary bus, master device only.** No
  IRQ-driven transfers, no secondary bus, no ATAPI, no write support.
- **FAT32 driver is read-only, root-directory only, 8.3 names only.**
  No subdirectories, no long filenames (LFN entries are skipped, not
  parsed), no write/create/delete.
- **"VFS" is a thin single-filesystem pass-through**, not a real
  mount-point table - see `kernel/fs/vfs.c`'s header comment.

## Phase 4 and beyond

Not started. See PROJECT_PLAN.md section 6 for scope.
