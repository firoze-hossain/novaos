# NovaOS - Progress

This file tracks what is *actually implemented and boot-tested*, as
opposed to PROJECT_PLAN.md, which tracks what's *intended*. Update this
in the same PR as the code it describes.

## Status at a glance

| Phase | Status |
|---|---|
| P1 - Bootloader & Kernel Foundation | Complete |
| P2 - Memory Management & Interrupts | Complete |
| P3 - Filesystem & Drivers | Not started |
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

## Phase 3 and beyond

Not started. See PROJECT_PLAN.md section 6 for scope.
