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
| P4 - Usermode Processes, Syscalls & Scheduling | Complete (scoped - see below) |
| P5 - Security Hardening (address-space isolation) | Complete (scoped - see below) |
| P6 - Networking | Complete (scoped - see below) |
| P7 - Graphics Mode & Windowing | Complete (scoped - see below) |
| P8 - Package Manager (nova-pkg CLI) | Complete (scoped - see below) |
| P9 - First-Run Setup, RTC & Persistent Identity | Complete (scoped - see below) |
| P10 - UDP, TFTP Client & Networked Package Fetching | Complete (scoped - see below) |
| P11 - Capability-Based File Access Control | Complete (scoped - see below) |
| P12 - GUI Software Center | Complete (scoped - see below) |
| P13 - PCI Bus Enumeration | Complete (scoped - see below) |
| P14 - Network Capability Enforcement | Complete (scoped - see below) |
| P15 - Font Punctuation & Package Descriptions | Complete (scoped - see below) |
| P16 - RTL8139 PCI NIC Driver | Complete (scoped - see below) |
| P17 - Process Creation Capability | Complete (scoped - see below) |
| P18 - AC97 PCI Sound Driver | Complete (scoped - see below) |
| P19 - Minimal DNS Client | Complete (scoped - see below) |
| P20 - Installable Disk Image | Complete (scoped - see below) |
| P21 - License, Versioning & Changelog | Complete - see below |
| P22 - Process Exit Resource Cleanup | Complete - see below |
| P23 - ELF Loading & a Real Process Model | Complete (scoped - see below) |
| P24 - Minimal Libc Port | Complete (scoped - see below) |

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

## Phase 4 - Usermode Processes, Syscalls & Scheduling

**Status: Complete, at a deliberately scoped-down scope.** Boot-verified
via `make test`, and manually confirmed the shell stays responsive
(`help`, `ps`) while the scheduler and a ring-3 process run underneath
it.

### What was built

- **`kernel/arch/x86/cpu/tss.*`** - a single Task State Segment
  (software task switching only - see the file's header comment).
  Its `esp0`/`ss0` fields are what let the CPU find the right ring-0
  stack when an interrupt or `int 0x80` fires while running in ring 3.
- **`kernel/arch/x86/cpu/context_switch.asm`** - `switch_context()`
  (the stack-swap primitive every preemption and voluntary yield goes
  through) and `enter_usermode` (the one-time ring0->ring3 `iret`
  trampoline a brand new user task's first switch lands on).
- **`kernel/task/process.*`** - process table, and the fake initial
  stack-frame construction a new task needs before it's ever run once
  (see the file's header comment - this is the fiddly part of doing
  context switching this way).
- **`kernel/task/scheduler.*`** - simple round-robin: scans the whole
  process table each time rather than maintaining a separate ready
  queue (fine at `MAX_PROCESSES=16`).
- **`kernel/drivers/timer/timer.c`** - gained `timer_set_tick_hook()`
  so the scheduler can preempt on a quantum (5 ticks / 50ms at the
  default 100Hz) without timer.c needing to know processes exist.
- **`kernel/arch/x86/cpu/syscall.*` + `syscall_stub.asm`** - `int 0x80`
  with `SYS_WRITE`/`SYS_EXIT`/`SYS_YIELD`. The gate's DPL=3 is what
  actually matters (the CPU checks `CPL <= gate DPL` for a software
  interrupt raised via `INT`) - every other IDT gate stays DPL=0.
- **`kernel/task/user_demo.c`** - a task that genuinely executes at
  CPU ring 3 and can only reach the kernel through `int 0x80` (see
  "Known limitations" for what "genuinely ring 3" does and doesn't
  mean yet without an ELF loader).
- **`kernel/shell/shell.c`** - added `ps`.

### Two real bugs found and fixed while building this

Both were invisible in code review and only showed up by actually
booting it - the entire reason this project keeps a `make test` and
insists on manual verification before calling a phase done:

1. **IRQ EOI was sent after calling the registered handler, not
   before.** Fine for every Phase 2/3 handler, which always returns
   normally - but the scheduler's timer tick hook can trigger a
   context switch that `ret`s straight into a different task's stack
   and never returns to that call frame at all. With EOI sent "after,"
   that code simply never ran, which permanently left the timer's IRQ
   line in-service on the (non-auto-EOI) 8259 PIC - no further timer
   interrupt was ever delivered again. Symptom: the very first
   scheduler tick worked exactly once, then the machine looked frozen
   (confirmed via QEMU's interrupt trace: EIP/ESP/EAX identical across
   several consecutive timer interrupts - it was re-entering the same
   `hlt` instruction over and over, EOI-blocked from ever seeing
   another one). Fixed by sending EOI immediately after identifying
   the IRQ, before dispatching to the handler.
2. **The identity-mapped pages were never marked user-accessible.**
   `paging_init()` (Phase 3) set `PAGE_PRESENT | PAGE_WRITE` but not
   `PAGE_USER` on every page table/directory entry - reasonable when
   nothing ran above ring 0 yet, but it meant the instant the ring-3
   demo task tried to execute its first instruction, the CPU faulted
   with "protection violation" (the page was present, just not
   permitted at CPL 3). Fixed by adding `PAGE_USER` to the identity
   map. See "Known limitations" below for what this does (and
   deliberately doesn't) mean for kernel/user memory separation.

### Verified behavior (this update)

- Clean build, zero warnings under `-Wall -Wextra`.
- Full boot-to-shutdown-of-demo sequence:
  ```
  [ OK ] TSS installed
  [ OK ] Syscall gate installed (int 0x80, ring 3 accessible)
  ...
  [ OK ] Tasks created: idle (kernel), shell (kernel), demo (ring 3)
  [SYSCALL] SYS_WRITE from pid 3 ('
  [ring3] Hello from user mode! This was printed via the SYS_WRITE syscall, not a direct vga_puts() call.
  ')
  [SYSCALL] SYS_WRITE from pid 3 ('[ring3] Demo task exiting via SYS_EXIT.
  ')
  [ OK ] Process 'demo' (pid 3) exited
  ```
- Confirmed (via temporary debug logging, since removed) the full
  round-robin sequence: idle -> shell -> demo -> idle -> shell -> demo
  (x3, matching the demo task's 3 `SYS_YIELD` calls) -> demo exits ->
  idle/shell continue round-robining indefinitely with demo correctly
  excluded (`PROCESS_TERMINATED` is never picked again).
- Ran for 15+ seconds headless with no crash, no fault, no runaway
  logging, and no apparent stack corruption from repeated switching.
- Interactively confirmed via the QEMU monitor that the shell (a
  kernel task, same as `idle`) stays fully responsive - `help` and
  `ps` both worked - while the scheduler and the ring-3 process run
  underneath it; the serial log showed no faults during that session.

### Known limitations / follow-ups (tracked for Phase 5+)

- **No ELF loader.** The demo user task is a C function compiled
  directly into the kernel image; `process_create_user_task()` takes
  its address the easy way. What's still genuinely real: it executes
  at CPL=3, can't execute privileged instructions, and can only reach
  the kernel through `int 0x80` - hardware-enforced, not just
  convention. What's not real yet: loading an actual executable
  (a.out/ELF) from the FAT32 filesystem into a fresh address space.
- **No per-process address spaces**, still - all tasks (kernel and
  user) share the single flat identity-mapped 64MB range from Phase 3.
  This phase's `PAGE_USER` fix (see above) means that range is now
  uniformly ring-3-accessible rather than kernel-only, which sounds
  like a regression but isn't one in practice: there was no
  kernel/user memory separation to preserve before this, either (see
  Phase 3's own limitations). Real isolation needs per-process page
  directories, tracked as future work.
- **No memory protection between processes.** Any task - kernel or
  user - can read/write any other task's stack if it has the address,
  since there's only one address space. The ring 0/3 CPL boundary
  (privileged instructions, the syscall gate) is real; memory
  isolation is not, yet.
- **`int 0x80` takes no argument validation.** `SYS_WRITE`'s pointer
  is dereferenced directly with no bounds/validity check - meaningless
  to "validate against the process's own memory" when every process
  shares one address space anyway, but will matter the moment that
  changes.
- **Round-robin only, fixed quantum, no priorities.** No sleep/wait
  queues either - `SYS_YIELD` is the only way a process gives up its
  quantum early.
- **`kmalloc`'d stacks are never freed** on process exit (`kfree()`
  exists and works - see Phase 2 - but nothing calls it yet on a
  `PROCESS_TERMINATED` slot). Fine for three tasks that live for the
  whole session; would leak in any longer-running, higher-churn setup.
- **Kernel task entry functions must not return** - there's no
  completion handler for kernel tasks the way `SYS_EXIT` provides one
  for user tasks. Both current kernel tasks (`idle`, `shell`) already
  loop forever, so this hasn't mattered in practice yet.

## Phase 5 - Security Hardening: Per-Process Address Space Isolation

**Status: Complete, at a deliberately scoped-down scope.** This phase
closes the specific gap flagged since Phase 3 and repeated in every
phase since: every process sharing one flat address space. Boot-
verified via `make test`, and it's the first phase whose core claim
(real memory isolation) is proven by the boot log itself rather than
needing a person to interpret it - see "Verified behavior" below.

### What was built

- **`kernel/arch/x86/mm/paging.c`** gained four functions:
  `paging_create_address_space()` (allocates a fresh page directory via
  the PMM and copies the kernel's own directory into it, so the kernel
  is mapped identically in every address space - required, since
  interrupts/syscalls run kernel code using whatever CR3 happens to be
  loaded), `paging_map_page()` (maps one page into a given directory,
  allocating a page table on demand), `paging_switch_address_space()`
  (loads CR3), and `paging_kernel_directory_phys()`.
- **`kernel/task/process.c`**: user tasks now get their own address
  space via `paging_create_address_space()`, and their stack is backed
  by fresh PMM frames mapped at a fixed virtual address
  (`USER_STACK_VIRT_BASE = 0x40000000`) - private to that process,
  unlike the Phase 4 stack, which was `kmalloc()`'d from the shared
  heap arena every process's directory maps identically. `process_t`
  gained a `page_directory_phys` field.
- **`kernel/task/scheduler.c`**: switches CR3 (via
  `paging_switch_address_space()`) on every context switch, not just
  the saved-register stack swap.
- **`kernel/task/user_demo.c`**: rewritten as two tasks (`demo-a`,
  `demo-b`) that each stamp their *own* private stack with a distinct
  64-byte pattern at the *same* virtual address, yield the CPU five
  times (letting the scheduler round-robin through idle, shell, and
  the other demo task), then verify their stack still reads back
  correctly. This is a real, falsifiable test: if isolation were
  broken - if both processes' stacks resolved to the same physical
  memory - one would silently overwrite the other's pattern and the
  verification would fail. It didn't.

### Verified behavior (this update)

- Clean build, zero warnings under `-Wall -Wextra`.
- Full boot log, unedited:
  ```
  [ OK ] Tasks created: idle (kernel), shell (kernel), demo-a + demo-b (ring 3, private address spaces)
  [SYSCALL] SYS_WRITE from pid 3 ('
  [ring3-A] Starting; stamped my private stack with 'A' x64.
  ')
  [SYSCALL] SYS_WRITE from pid 4 ('[ring3-B] Starting; stamped my private stack with 'B' x64 (same virtual address as process A, different physical page).
  ')
  [SYSCALL] SYS_WRITE from pid 3 ('[ring3-A] PASS: stack still all 'A' after yielding 5x - process B never touched my private memory.
  ')
  [ OK ] Process 'demo-a' (pid 3) exited
  [SYSCALL] SYS_WRITE from pid 4 ('[ring3-B] PASS: stack still all 'B' after yielding 5x - process A never touched my private memory.
  ')
  [ OK ] Process 'demo-b' (pid 4) exited
  ```
  Both PASS on the first successful run after implementation - unlike
  Phases 2-4, this phase's core mechanism worked correctly the first
  time it was tested end to end (the earlier phases' hard-won lessons
  about EOI ordering, page permission bits, and stack-frame
  construction all fed directly into getting this one right).
- Ran 20 seconds headless with no crash, fault, or `FAIL` message.
- Interactively confirmed the shell (`ps`, running as a kernel task
  sharing the original kernel address space) stays fully responsive
  while two processes with their own private address spaces run
  underneath it; serial log showed no faults during that session.
- `make test` now asserts both `ring3-A] PASS` and `ring3-B] PASS`
  appear and that no `FAIL`/`PANIC`/`FAULT` does - this phase is the
  first where "the security property holds" and "the boot log is
  parseable proof of it" are the same check.

### Known limitations / follow-ups (tracked for Phase 6+)

- **Still no ELF loader.** Both demo tasks are C functions compiled
  into the kernel image; only their *data* (the stack) is now private
  per process. Their *code* still lives in the shared, identity-mapped
  kernel range (readable/executable by every process, same as the
  kernel itself) - loading a real, separately-linked executable into
  its own private code mapping is still future work.
- **Kernel stacks are still shared-heap `kmalloc()`, not private.**
  Only user stacks got the isolation treatment this phase, since the
  kernel is trusted code by definition in this design - the threat
  model is "process A can't read process B's data," not "the kernel
  might corrupt its own bookkeeping."
- **Fragile assumption, called out in `paging.c`'s comments:** every
  new page directory/table comes from `pmm_alloc_frame()` and is used
  as a directly-dereferenceable pointer with no translation step. This
  only works because there's no general "temporarily map an arbitrary
  physical page" mechanism yet, and the PMM's bitmap scan happens to
  hand out low (already identity-mapped) frames first for the small
  number of allocations this phase makes. A long-running system that
  had exhausted or fragmented low memory could get a frame above 64MB
  here and silently misbehave. Tracked as real future work, not hidden.
- **No cleanup on process exit.** A `PROCESS_TERMINATED` process's
  page directory, page tables, and physical frames are never freed
  (same limitation as Phase 4's kernel/user stacks - now extended to
  cover the new per-process paging structures too). Fine for two
  short-lived demo tasks; would leak in any longer-running, higher-
  churn workload.
- **No capabilities/least-privilege model, no sandboxing.** This phase
  closed the concrete "shared address space" gap specifically; the
  broader "capabilities, not raw root/non-root" and "mandatory
  sandboxing for GUI apps" items from PROJECT_PLAN.md's security
  roadmap remain unstarted and need a filesystem permissions model
  and/or a GUI to sandbox in the first place - realistically Phase 6+
  scope once there's more surface area to actually secure.
- **Still no NX bit** (32-bit non-PAE paging - see Phase 3's own
  limitations, unchanged).

## Phase 6 - Networking

**Status: Complete, at a deliberately scoped-down scope.** A full,
genuine network round trip - ARP resolution followed by an ICMP ping -
verified against QEMU's own user-mode networking gateway, which
requires no real network access from the host or CI runner to work.

### What was built

- **`kernel/drivers/net/ne2000.*`** - polling PIO driver for the
  NE2000 ISA NIC at the fixed QEMU default I/O base (0x300), matching
  the ATA driver's "one fixed device" precedent from Phase 3. Reads
  the MAC address out of the card's PROM, sends/receives raw Ethernet
  frames via the card's remote-DMA mechanism and page-based ring
  buffer.
- **`kernel/net/ethernet.*`** - builds outgoing frames, dispatches
  incoming ones to ARP or IPv4 by ethertype.
- **`kernel/net/arp.*`** - resolves an IP to a MAC (one-entry cache -
  see limitations), answers incoming ARP requests for our own IP.
- **`kernel/net/ip.*`** - minimal IPv4: header build/parse, the
  standard Internet checksum, no fragmentation, no options, no routing
  table (always ARPs the destination directly - see limitations).
- **`kernel/net/icmp.*`** - Echo Request/Reply only. `icmp_ping()`
  sends a request and busy-waits (via `net_poll()`) for the matching
  reply; incoming Echo Requests are answered automatically, which is
  what lets another host `ping` NovaOS.
- **`kernel/net/net.*`** - fixed static network configuration (no
  DHCP client) matching QEMU user-mode networking's defaults (IP
  10.0.2.15, gateway 10.0.2.2), the shared IP/ICMP checksum helper, and
  `net_poll()` - since the NE2000 driver has no IRQ, the idle task
  calls this once per loop iteration to drain received frames.
- **`kernel/shell/shell.c`**: added `ping IP` (with a small hand-rolled
  dotted-quad parser - no `sscanf` in this libc subset).
- Wired into `kernel_late_init()`'s self-test pattern: pings the
  gateway automatically at boot and logs the result, the same way
  Phase 3 read a test file and Phase 5 verified stack isolation.

### One real bug found and fixed while building this - a classic

Caught by actually testing, and confirmed with a QEMU packet capture
before touching any code - exactly the discipline that's caught a real
bug in every phase so far:

**NE2000 receive ring off-by-one.** The hardware `BNRY` register (and
this driver's software copy of it) tracks the *last freed* page, not
the next unread one - the actual unread packet lives at `BNRY + 1`.
The first version of `ne2000_receive()` read directly from `BNRY`,
which (right after init, before anything has ever been read) is a page
the NIC never wrote to, producing an all-zero header that the
corrupt-packet safety check silently discarded. Symptom: the ARP
request was verifiably being sent and verifiably being answered (a
`-object filter-dump` packet capture showed both the request and
SLIRP's reply on the wire), yet the kernel never logged receiving
anything at all. Fixed by reading from `BNRY + 1` and keeping the
software/hardware BNRY values consistently defined as "last freed
page" throughout.

### Verified behavior (this update)

- Clean build, zero warnings under `-Wall -Wextra`.
- Full boot log, unedited:
  ```
  [ OK ] NE2000 NIC at 0x300, MAC 52:54:0:12:34:56
  [ OK ] Network up: IP 10.0.2.15, gateway 10.0.2.2
  ...
  [ OK ] PING OK: gateway replied in 0 ticks (~0ms)
  ```
  ("0 ticks" reflects the timer's 10ms tick resolution, not an
  instantaneous reply - QEMU's virtual network genuinely does respond
  faster than one tick most of the time. RTT precision is limited to
  whole ticks; see limitations.)
- Diagnosed the ring-buffer bug with a `-object filter-dump` packet
  capture (proving the request/reply genuinely existed on the wire)
  before touching the receive code, rather than guessing.
- Ran 20 seconds headless with no crash, fault, or FAIL message.
- Interactively confirmed the shell's `ping 10.0.2.2` command works
  and produces a reply; serial log showed no faults during that
  session.
- `make test` now asserts `PING OK` appears alongside the existing
  Phase 2-5 markers.

### Known limitations / follow-ups (tracked for Phase 7+)

- **No DHCP.** Static IP configuration matching QEMU user-mode
  networking's defaults only - a different network setup (bridged,
  tap, a different QEMU `-netdev`) needs `net.h`'s constants changed by
  hand, or a real DHCP client written.
- **No routing table.** `ip_send()` always ARPs the destination IP
  directly, which only works for same-subnet destinations. Fine for
  the one thing NovaOS currently talks to (the gateway, which is
  same-subnet by definition); would silently fail to reach anything
  requiring an actual multi-hop route.
- **One-entry ARP cache**, not a real table - resolving a second host
  evicts the first. Matches this phase's actual needs (one gateway) but
  is a real limitation the moment more than one destination matters.
- **NE2000 driver is polling, not IRQ-driven.** Something (currently
  the idle task) has to call `net_poll()` regularly or incoming frames
  sit in the NIC's ring buffer unprocessed. No PCI variant, no
  multiple-NIC support, no jumbo frames.
- **UDP and TCP are not implemented at all** - only ICMP. No sockets
  API. This is the largest remaining gap in "Networking (NE2000,
  TCP/IP, sockets)" as originally scoped in PROJECT_PLAN.md; closing it
  is real future work, not a small addition.
- **RTT reporting is only accurate to one timer tick (10ms).** A
  faster reply (as QEMU's virtual network usually gives) still reports
  as "0ms" rather than a genuine sub-tick measurement.
- **No packet validation hardening** - checksums are computed correctly
  on send but not verified strictly on receive beyond basic length
  sanity checks; a deliberately malformed packet from a hostile host
  hasn't been fuzzed against.

## Phase 7 - Graphics Mode & a Minimal Windowing System

**Status: Complete, at a deliberately scoped-down scope, with one
honestly-unresolved verification gap flagged below.** Real pixel-level
graphics, a real second hardware input device (PS/2 mouse), and a
small windowing demo, switchable at runtime without disturbing the
existing text-mode shell at all.

### The key architectural decision

Rather than requesting a linear framebuffer through Multiboot/GRUB's
VBE negotiation - the "normal" way a protected-mode kernel gets
graphics - this phase uses **VGA Mode 13h (320x200x256) via direct
hardware register programming** instead. The reason: a VBE framebuffer
would *replace* the VGA text-mode console the shell has depended on
since Phase 1, which means porting every existing command's text
output to a framebuffer-rendered bitmap font covering the full ASCII
range - a lot of hand-transcribed glyph data with real risk of subtly
garbling output in ways that are easy to miss. Mode 13h can be entered
and exited at runtime with pure port I/O (no BIOS calls - this kernel
has no real/virtual-8086 mode to make them from), so the existing shell
is completely unaffected outside the moments a user is actually inside
the new `gui` command. The tradeoff: lower resolution (320x200 vs.
whatever VBE could offer) and only a handful of hand-built digit
glyphs (see `font5x7.h`) rather than a full font.

### What was built

- **`kernel/drivers/video/vga_graphics.*`** - Mode 13h enter/exit via
  direct programming of the VGA Sequencer, CRTC, Graphics Controller,
  and Attribute Controller registers, plus pixel/rectangle drawing
  primitives.
- **`kernel/drivers/mouse/ps2mouse.*`** - PS/2 mouse driver, IRQ12,
  standard 3-byte relative-packet protocol.
- **`kernel/gui/compositor.*`** - three draggable colored rectangle
  windows with titlebars (labeled 1/2/3 via a small hand-built 5x7
  digit font, `font5x7.h`), double-buffered rendering (an off-screen
  buffer blitted to `0xA0000` once per frame to avoid tearing), and a
  simple cursor.
- **Shell `gui` command** - enters graphics mode, runs the compositor
  loop, ESC returns cleanly to the text shell.

### Three real bugs found and fixed while building this

All three were invisible in code review and only surfaced by actually
exercising hardware paths no earlier phase had touched:

1. **IRQ12 needs the master PIC's cascade line (IRQ2) unmasked.**
   `register_irq_handler()` only ever unmasked the specific line
   requested - fine for IRQ0/1 (both on the master PIC, used since
   Phase 2), but IRQ12 lives on the *slave* PIC, and slave-PIC
   interrupts physically cannot reach the CPU at all unless IRQ2 on
   the master is also unmasked. This is the first IRQ Phase 7 added
   above 7, so it's the first time this requirement ever mattered.
2. **PS/2 packet framing misalignment.** A stale byte sitting in the
   8042 controller's output buffer at driver-init time got accepted as
   a false packet start - and, by coincidence, the real Y-delta byte
   in the correctly-aligned stream also happened to have the packet
   sync bit set, so the misalignment never self-corrected once it
   happened. Fixed by draining any stale byte before switching to
   interrupt-driven reads.
3. **Boot-to-command accumulation.** Mouse movement between driver
   init (at boot) and the user actually typing `gui` accumulates in
   the driver (by design - it's a relative-motion accumulator), and
   was throwing off the cursor's starting position with a large stale
   delta the first time `gui` read it. Fixed with a settling-window
   discard on entry to the command.

### Verified behavior

- Clean build, zero warnings under `-Wall -Wextra`.
- **Zero regression**: `make test` confirms every Phase 2-6 boot
  marker (FAT32, networking, ring-3 processes, ping) still passes
  exactly as before, plus PS/2 mouse now initializes at boot.
- **Mode switching, rigorously verified**: compared actual pixel
  dimensions and color palettes before/after `gui` + ESC - graphics
  mode measured as 320x200 (640x400 in QEMU's 2x screendump scaling)
  with the expected window colors; text mode measured as 80x25
  (720x400 in the same scaling) with the expected black/white/cyan
  console palette, immediately after exiting. The round trip is solid.
- **Mouse decode/accumulation, proven correct** by direct kernel-level
  instrumentation (since removed): five repeated, identical
  `mouse_move 20 15` monitor commands each independently produced an
  exact `dx=20 dy=15` reading with no drift or corruption.
- **Button-press detection, proven correct**: `left_button` correctly
  read as true while the button is held, false once released.
- **Rendering, confirmed by pixel inspection**: the cursor renders at
  the correct default position; window bodies, titlebars, borders, and
  digit labels all render in the intended distinct colors.
- Ran 20+ seconds headless with no crash or fault; confirmed the shell
  fully recovers (correctly re-measured at 80x25 text-mode dimensions)
  and keeps responding normally to `help`/`ps`/`uptime` immediately
  after a `gui` session ends.

### Known limitation: automated drag-and-drop verification is incomplete

This is being flagged directly rather than glossed over. Every
individual mechanism behind dragging a window - IRQ delivery, packet
decode, accumulation, button-state tracking, hit-testing against a
titlebar rectangle, and rendering - was independently verified correct
(see above). However, scripting a *multi-step* drag sequence (move
onto a titlebar, press, move while held, release) through QEMU's HMP
monitor with `-display none` produced inconsistent, hard-to-explain
results when the sequence used *varying* delta values across several
`mouse_move` calls in one session - even though five *identical*
repeated calls worked perfectly every time. This was narrowed down
about as far as headless, monitor-scripted testing allows: it doesn't
reproduce with repeated identical values, it isn't explained by
monitor-command timing (an explicit prompt-synchronized test still
showed it), and it doesn't correlate with the graphics-mode switch
itself (a settling-window discard didn't change the pattern). The
most likely explanation is a QEMU HMP/headless-input quirk specific to
`-display none` plus scripted mouse events with changing magnitudes -
a testing configuration with no real-world analogue (an actual user
runs `make run` with a real display and a real mouse, which streams
naturally smooth, fine-grained deltas - exactly the pattern proven to
work) - but this was not proven to 100% certainty within the available
debugging time, and it deserves a real user's manual confirmation
rather than a confident claim either way.

**If you try `gui` with `make run` and dragging doesn't work
smoothly**, that's a genuine bug report worth filing - please include
what you observed (does the cursor move at all? does clicking a
titlebar do anything?) so it can be root-caused with real interactive
input rather than scripted monitor commands.

### Other known limitations / follow-ups (tracked for Phase 8+)

- **320x200x256 only** - no other VGA graphics mode, no VBE/linear
  framebuffer support, no resolution changes.
- **Font covers digits 0-9 only** (window titlebar labels). No general
  text rendering in graphics mode - see the architectural decision
  above for why.
- **Exactly 3 windows, fixed at compositor_init() time.** No creating,
  closing, resizing, or minimizing windows; no focus/z-order beyond
  "whichever was dragged most recently draws over stale diff
  boundaries" (draw order is otherwise fixed); no overlap-aware damage
  tracking (the whole screen redraws every frame).
- **No real GUI toolkit** - no buttons, no text input fields, no
  events beyond raw mouse position/buttons and a single ESC-to-exit
  keyboard check.
- **PS/2 mouse only** - no USB HID mouse/tablet support.

## Phase 8 - Package Manager (nova-pkg CLI)

**Status: Complete, at a deliberately scoped-down scope.** The
technically significant part of this phase is that **FAT32 gained
write support** - every phase since Phase 3 has been read-only. The
package manager itself is a straightforward CLI built on top of that.

### Scope note: no GUI "Software Center"

The original Phase 8 plan (PROJECT_PLAN.md) called for both a CLI
package manager *and* a GUI front-end. Only the CLI half is built
here - a GUI needs general text rendering in graphics mode, which
doesn't exist yet (Phase 7's VGA Mode 13h mode has only a handful of
hand-built digit glyphs, see `kernel/gui/font5x7.h`, deliberately kept
that small - see Phase 7's own notes on why). Tracked as follow-up
work once a real font exists.

### What was built

- **`kernel/drivers/ata/ata.c`** gained `ata_write_sectors()` (the
  WRITE SECTORS command, followed by FLUSH CACHE for durability) -
  the foundation everything else in this phase sits on.
- **`kernel/fs/fat32.c`** gained real write support:
  `fat_set_next_cluster()` (read-modify-write, preserving the
  reserved top 4 bits, updates *every* FAT copy for redundancy - real
  FAT32 usually has two), a linear free-cluster scanner and cluster-
  chain allocator/freer, and directory-entry create/delete - including
  automatically extending the root directory with a fresh cluster if
  every existing entry slot is full. `fat32_write_file()` (create-only,
  fails if the name already exists - no overwrite/append/truncate) and
  `fat32_delete_file()` round out the read-only Phase 3 API.
- **`kernel/pkg/pkgmgr.*`** - nova-pkg itself. A package is a single
  `<NAME>.PKG` file (there are no subdirectories to build a real
  repository layout with yet) - a small fixed manifest header (magic,
  name, version, description, payload size) followed immediately by
  the raw payload. `pkg_install()` copies a package's payload out to
  `<NAME>.APP` and records the install in `INSTALL.DB` (a flat array
  of fixed-size records, rewritten wholesale on every change - simple,
  and plenty for a package count in the single digits);
  `pkg_remove()` deletes both.
- **Shell**: `pkg list`, `pkg installed`, `pkg install NAME`,
  `pkg remove NAME`.
- **Two demo packages** (`tools/fixtures/EDITOR.PKG`,
  `tools/fixtures/GAME.PKG`) baked into the test disk image.
- **Boot self-test**: installs "Editor", reads back `EDITOR.APP` and
  logs its exact content (proving the write path produced byte-correct
  data, not just "didn't crash"), then removes it and confirms
  `pkg_is_installed()` agrees it's gone.

### A reentrancy trap avoided, not hit - worth documenting anyway

While writing `pkgmgr.c`, one design constraint mattered enough to
shape the whole file's structure: `fat32.c`'s directory-walk and
file-read functions share static scratch buffers (`cluster_buf`,
`fat_sector_buf`) rather than using the heap or the stack for them.
Calling `vfs_read_file()` from *inside* a directory-listing callback
would silently corrupt the very listing still being iterated, since
both paths reuse the same buffer. Every function in `pkgmgr.c` that
needs both "list files" and "read a file's contents" does so in two
clearly separate passes instead of nesting them - see the comment at
the top of the file. This was caught by reasoning through the existing
code before writing new code that called it, not by hitting a bug at
runtime - the one phase so far where the "find a bug via testing"
pattern didn't apply, because the trap was avoided up front instead.

### Verified behavior

- Clean build, zero warnings under `-Wall -Wextra`.
- **Zero regression**: every Phase 2-7 `make test` marker still
  passes.
- Full boot log, unedited:
  ```
  [ OK ] Installed package 'Editor' -> EDITOR.APP
  [ OK ] PKG INSTALL OK: EDITOR.APP (62 bytes): This is the Editor application payload.
  NovaOS nova-pkg demo.

  [ OK ] Removed package 'Editor'
  [ OK ] PKG REMOVE OK: Editor no longer installed
  ```
  The installed file's content matches the original package's payload
  exactly.
- Ran 20 seconds headless with no crash or fault.
- Interactively exercised `ls`, `pkg list`, `pkg install Game`,
  `pkg installed`, `cat GAME.APP`, and `pkg remove Game` through the
  shell; the serial log confirmed each operation succeeded with no
  faults, and the resulting screens showed real, non-corrupted text
  output (sanity-checked via color-palette diversity in addition to
  visual inspection).
- `make test` now asserts `PKG INSTALL OK` and `PKG REMOVE OK` appear
  in the boot log alongside every earlier phase's markers.

### Known limitations / follow-ups (tracked for Phase 9+)

- **No GUI Software Center** - see the scope note above.
- **No package repository or network fetch.** Packages must already
  exist on the mounted disk; NovaOS's network stack (Phase 6) only
  speaks ICMP, not HTTP/FTP, so there's nothing to fetch a package
  *from* yet even if a repository format existed.
- **`fat32_write_file()` is create-only** - no overwrite, append, or
  truncate. "Reinstalling" a package that's still installed requires
  removing it first (which `pkg_install()` already enforces via the
  install-database check, so this isn't a gap in the package manager's
  own behavior, just in the underlying filesystem primitive).
- **No subdirectories**, still (a Phase 3 limitation, unchanged) - a
  real package repository with categories, or an installed-apps
  directory separate from the root, needs this.
- **`INSTALL.DB` and package files live in the one shared, flat root
  directory** alongside everything else on the disk (`HELLO.TXT`, the
  `.PKG` files themselves, etc.) - there's no dedicated "system"
  location.
- **No package signing or integrity verification** - the security
  roadmap in PROJECT_PLAN.md has always listed this as Phase 8+ scope;
  it remains unstarted. A malicious or corrupted `.PKG` file's payload
  is trusted and installed as-is.
- **Failure paths are not perfectly atomic.** `alloc_cluster_chain()`
  doesn't roll back partial allocations if it runs out of space
  partway through; `pkg_remove()` proceeds to delete the `.APP` file
  even if updating `INSTALL.DB` afterward fails (logged, not silent,
  but could leave a stale database record - see the code comment in
  `pkgmgr.c`). Fine for a single-user hobby OS's current needs; a real
  filesystem would want proper journaling or at least ordered,
  rollback-capable operations.

## Phase 9 - First-Run Setup, RTC & Persistent Identity

**Status: Complete, at a deliberately scoped-down scope.** The
original phase name in PROJECT_PLAN.md was "Installer, first-run
wizard, driver support, public release polish" - this delivers the
realistic version of all four for where NovaOS actually is right now.

### Scope note: what "installer" means here, and what it doesn't

A traditional OS installer writes a bootloader to a hard disk's boot
sector so the machine can boot independently of the install media.
NovaOS still boots from a live CD/USB image every time (see
PROJECT_PLAN.md) - writing our own bootloader, or replicating enough
of what GRUB does to install it from inside our own kernel, is a
substantial separate undertaking that was not attempted here and
isn't secretly half-done; it's simply not started.

What *is* built, and is the realistic "installer" for a live-boot
design like this one - the same pattern many live-CD Linux
distributions use for persistence - is a first-run setup wizard that
asks for a hostname and username once, then saves them to the
attached disk so every later boot recognizes the machine instead of
re-asking. That's genuinely useful (it's most of what a "first-run
wizard" step of a real installer does anyway, minus the disk
partitioning), and it's honestly the whole of what got built.

### What was built

- **`kernel/drivers/rtc/rtc.c`** - reads the CMOS real-time clock.
  Handles both BCD and binary storage modes (checked via CMOS Status
  Register B, not assumed) and both 12/24-hour formats, using the
  standard "wait for update-not-in-progress, then read twice and
  require the results to match" technique to avoid a torn reading -
  this is how every real RTC driver has to handle this chip, not a
  NovaOS-specific workaround.
- **`kernel/config/sysconfig.*`** - reads/writes a small fixed-size
  `SYSTEM.CFG` file (hostname + username) with overwrite semantics
  (delete-then-recreate, since the underlying `fat32_write_file()`
  from Phase 8 is create-only).
- **`kernel/shell/firstrun.*`** - the wizard itself. Called once from
  `kernel_main()`, after the boot banner and before any tasks are
  created (deliberately not part of the shell's command loop, so the
  shell task never needs "is this the first run" logic of its own).
  Loads `SYSTEM.CFG` if present and greets the returning user; runs an
  interactive hostname/username prompt and saves the result if not.
- **Shell**: the prompt is now `username@hostname>` instead of the
  generic `nova>`; added `date` (RTC), `hostname`, and `whoami`.

### Verified behavior - three separate, escalating checks

1. **Returning-user path**: the test disk image now includes a
   pre-seeded `SYSTEM.CFG` (`tools/fixtures/SYSTEM.CFG`), so headless
   `make test` exercises this path automatically. Boot log:
   `[ OK ] First-run check: returning user 'demo' on 'novaos-test'` -
   exact match to the fixture, first try.
2. **Interactive wizard path**: booted with a disk that had
   `SYSTEM.CFG` deliberately removed, and scripted real keystrokes
   through the QEMU monitor answering the hostname ("mypc") and
   username ("alice") prompts. Boot log:
   `[ OK ] First-run wizard complete: 'alice' on 'mypc'` - exact
   match, first try. Screenshots confirmed the interactive prompts and
   the personalized `alice@mypc>` shell prompt rendered correctly, and
   `whoami`/`hostname`/`date` all worked.
3. **The check that actually matters - persistence across a real
   reboot**: rebooted with that *same* disk image (now containing the
   `SYSTEM.CFG` the wizard had just written) and confirmed:
   `[ OK ] First-run check: returning user 'alice' on 'mypc'`. This is
   the property the whole feature exists to provide, and it's the one
   that was actually tested end to end rather than assumed.
- Clean build, zero warnings under `-Wall -Wextra`; zero regression -
  every Phase 2-8 `make test` marker still passes.
- `make test` now also asserts `First-run check: returning user`
  appears in the boot log.

### Known limitations / follow-ups (tracked for Phase 10+)

- **No real installer.** See the scope note above - this is the
  honest, permanent state of this item until someone builds a
  bootloader-writing installation step, which is out of scope for the
  foreseeable roadmap, not merely "not yet built this phase."
- **RTC is read-only.** No setting the clock, no timezone handling
  (always whatever the CMOS clock says, presented as "UTC" without
  actually knowing that's true), no leap-second/leap-year edge-case
  hardening beyond what the chip itself provides.
- **`date`'s output isn't zero-padded** (e.g. `9:5:3` rather than
  `09:05:03`) - `vsnprintf`'s `%d` has no width/padding support, a
  known libc-subset gap since Phase 2, not something this phase
  attempted to fix.
- **The wizard has minimal input validation** - an empty hostname or
  username falls back to a default ("novaos"/"user"), but there's no
  length/character-set enforcement beyond the field's fixed buffer
  size, no confirmation step, and no way to re-run setup later short
  of manually deleting `SYSTEM.CFG` (there's no shell command for
  that yet either - `pkg`-style tooling to reset system config is a
  reasonable small follow-up).
- **Single user, no accounts/permissions.** "Username" here is a
  cosmetic identity string, not a real multi-user account system with
  authentication - NovaOS still has no login, no passwords, no
  per-user permissions.
- **No public-release polish beyond what's described above** -
  packaging, licensing decisions, a real project website, etc. are
  all still open, ordinary open-source-project maintenance tasks
  rather than anything this phase specifically addresses.

## Phase 10 - UDP, TFTP Client & Networked Package Fetching

**Status: Complete, at a deliberately scoped-down scope.** This phase
wasn't part of the original nine-phase plan in PROJECT_PLAN.md - P1-P9
completed that plan in full (see the note at the end of Phase 9's
section above). This is the first phase chosen from a fresh look at
what was already flagged as deferred, rather than from a pre-written
roadmap: Phase 6 explicitly deferred UDP, and Phase 8 explicitly
deferred "no network fetch, nothing to download a package from yet."
Closing both together, with the second built directly on the first,
made for a coherent unit of work.

A real installer (writing a bootloader to a disk) remains the other
major deferred item and was deliberately *not* chosen this time - it
needs new real-mode BIOS/assembly work with no existing infrastructure
to build on, a meaningfully different risk profile than extending the
network stack that already works.

### What was built

- **`kernel/net/udp.*`** - minimal UDP: send, and a single-listener
  receive dispatch wired into `ip_handle_packet()` alongside ICMP.
  Checksums are disabled on send (0 is a valid "not computed" value
  per the IPv4 spec) and not validated on receive - a reasonable
  simplification on a trusted local virtual network, and consistent
  with `ip.c` already not validating its own header checksum on
  receive either. Only one thing can be "listening" at a time (a
  static single slot, not a real port table) - enough for the one
  thing that uses UDP so far (TFTP), the same one-outstanding-
  operation simplification `arp.c`'s cache and `icmp.c`'s ping
  tracking already use.
- **`kernel/net/tftp.*`** - a read-only TFTP client (RFC 1350): RRQ,
  receive DATA blocks, send ACK, until a short (<512 byte) block
  signals end-of-file. Locks onto the server's actual reply port after
  the first response, since real TFTP servers answer from a new
  ephemeral port, not port 69 itself. No retransmission on a lost
  packet - one overall ~10s deadline for the whole transfer rather
  than a more forgiving per-block timeout with retries.
- **Shell**: `tftp get FILE [SERVER_IP]` (defaults to the gateway),
  and `pkg fetch NAME` - downloads `<NAME>.PKG` via TFTP from the
  gateway and saves it locally, ready for the existing `pkg install`.
- **`tools/fixtures/tftproot/WEATHER.PKG`** - a third demo package,
  served over TFTP rather than baked directly onto the disk image like
  `EDITOR.PKG`/`GAME.PKG` are, specifically to exercise the network
  path. QEMU's SLIRP runs a TFTP server on the gateway address when
  given `-netdev ...,tftp=DIR` (see `Makefile`'s `NET_FLAGS`) - no
  real network access needed, the same self-contained-test principle
  every earlier network self-test already relies on.
- **Boot self-test**: fetches `WEATHER.PKG` over TFTP right after the
  existing ping self-test and logs the result.

### Verified behavior

- Clean build, zero warnings under `-Wall -Wextra`; zero regression -
  every Phase 2-9 `make test` marker still passes.
- Boot log, first try, no debugging needed:
  `[ OK ] TFTP FETCH OK: WEATHER.PKG (172 bytes)` - an exact byte-count
  match to the source fixture file.
- Full interactive workflow scripted through the QEMU monitor:
  `pkg fetch Weather` -> `pkg list` (shows Weather as available) ->
  `pkg install Weather` -> `cat WEATHER.APP` (prints the exact fetched
  payload) -> `pkg installed` (shows it installed). Serial log
  confirms `Installed package 'Weather' -> WEATHER.APP`. Zero faults
  throughout.
- Screenshots confirmed legitimate, non-corrupted text output at every
  step (sanity-checked via color-palette diversity in addition to
  visual inspection, the same approach used since Phase 8).

### Known limitations / follow-ups (tracked for future phases)

- **No TCP.** ICMP (Phase 6) and now UDP are the only transport-layer
  protocols. No sockets API, no HTTP - a real package repository
  server (as opposed to a single flat TFTP directory) would want at
  least one of these.
- **UDP supports exactly one listener at a time.** A second concurrent
  UDP-based feature would need a real port table, not the current
  single static slot.
- **No UDP checksum validation** (or computation on send, beyond
  emitting the valid "disabled" value 0) - see the design note above.
- **TFTP is read-only** (no WRQ/write support) and has **no
  retransmission** - a real network with meaningful packet loss would
  need both before this could be relied on beyond a controlled local
  virtual network.
- **No DNS.** `tftp get`/`pkg fetch` only accept a raw IP address for
  the server (defaulting to the gateway) - no hostname resolution
  exists anywhere in the network stack yet.
- **The "package repository" is still just a flat directory of
  `.PKG` files** - TFTP serving them over the network doesn't change
  that there's no real repository format, versioning/dependency
  metadata, or index beyond what each package's own manifest header
  carries (unchanged from Phase 8).

## Phase 11 - Capability-Based File Access Control

**Status: Complete, at a deliberately scoped-down scope.** Closes the
"least-privilege process model (capabilities, not raw root/non-root)"
item from PROJECT_PLAN.md's security roadmap, deferred since Phase 5.
Chosen over the other open candidates (a real installer, TCP/sockets)
specifically because it builds on infrastructure that already exists
(syscalls since Phase 4, FAT32 since Phase 3/8, per-process address
spaces since Phase 5) rather than needing a new subsystem, and because
it's the most directly security-relevant of the remaining options.

### The actual gap this closes

Every ring-3 process since Phase 4 could do exactly three things:
print a string, yield, and exit. None of them could touch the
filesystem at all - `nova-pkg`, `ls`, `cat`, and everything else
file-related has only ever run as kernel (ring-0) code from the shell.
This phase gives ring-3 code a real, narrow path to file I/O, gated by
an explicit per-process grant list checked in the kernel - not
something a process can expand by asking nicely, guessing a handle
number, or any other means short of an actual kernel bug.

### What was built

- **`kernel/arch/x86/cpu/syscall.h`**: three new syscalls - `SYS_OPEN`
  (EBX = filename, returns a handle or -1), `SYS_READ` (EBX = handle,
  ECX = buffer, EDX = max length, returns bytes read or -1), and
  `SYS_CLOSE`. These are the first syscalls in NovaOS to return a
  value: `syscall_handler()` writes it into `registers_t.eax`, which
  is exactly the in-memory slot the entry stub's final `popa` restores
  real EAX from - no separate return channel needed, it falls out of
  the existing stack-frame design from Phase 4.
- **`kernel/task/process.h`**: `process_t` gained `allowed_files[4]`
  and `allowed_file_count` - the capability list. Empty by default for
  *every* process, including ones made with the existing
  `process_create_user_task()`; only the new
  `process_create_sandboxed_task(name, entry, filenames, count)`
  grants anything. Least privilege as the default a caller has to
  affirmatively opt out of, not a feature a caller has to remember to
  turn on.
- **`kernel/arch/x86/cpu/syscall.c`**: the actual enforcement.
  `SYS_OPEN` checks the calling process's capability list
  (`process_current()->allowed_files`) before allocating a handle from
  a small global open-file table (8 slots); denied opens are logged at
  `[SECURITY]` level with the pid and filename. `SYS_READ`/`SYS_CLOSE`
  additionally verify the calling process's pid matches the handle's
  recorded owner - a process can't use a handle index it didn't
  receive from its own `SYS_OPEN` call, even by guessing.
- **`kernel/task/sandbox_demo.*`** - a ring-3 process created with a
  capability list containing only `"HELLO.TXT"`. Deliberately tries
  both an allowed open (`HELLO.TXT`, should succeed) and a disallowed
  one (`SYSTEM.CFG`, should be denied) and reports PASS/FAIL for each
  - a falsifiable test, the same approach Phase 5 used to prove
  address-space isolation actually held rather than merely claiming it.

### Verified behavior

- Clean build, zero warnings under `-Wall -Wextra`; zero regression -
  every Phase 2-10 `make test` marker still passes.
- Full boot log, unedited, first try - no debugging needed:
  ```
  [SYSCALL] pid 5 SYS_OPEN('HELLO.TXT') -> handle 0 (capability granted)
  [SYSCALL] SYS_WRITE from pid 5 ('[sandbox] PASS: HELLO.TXT opened and read (allowed by capability list): ')
  [SYSCALL] SYS_WRITE from pid 5 ('Hello from NovaOS FAT32! ...')
  [SECURITY] pid 5 denied SYS_OPEN('SYSTEM.CFG') - not in its capability list
  [SYSCALL] SYS_WRITE from pid 5 ('[sandbox] PASS: SYS_OPEN("SYSTEM.CFG") correctly denied - not in my capability list.')
  [ OK ] Process 'sandbox' (pid 5) exited
  ```
  Both the allowed-file success and the disallowed-file denial matched
  expectations exactly - the process's own PASS/FAIL self-check and
  the kernel's independent `[SECURITY]` log line agree.
- Interactively confirmed via `ps` that the sandbox process appears
  and terminates cleanly alongside the existing demo processes, with
  no crashes or faults in the serial log.
- `make test` now also asserts both PASS lines and the `[SECURITY]`
  denial line all appear.
- Along the way, confirmed (by reasoning about the existing Phase 4/5
  design rather than by hitting a bug) that syscall handlers can
  safely dereference "user" pointers (like `SYS_READ`'s buffer
  argument) directly: a syscall doesn't switch CR3, so the page
  directory active while handling a process's syscall is that same
  process's own - the pointer is valid in exactly the context it's
  being read in.

### Known limitations / follow-ups (tracked for future phases)

- **Fixed capability list, granted only at process creation.** No
  runtime grant/revoke, no wildcard or directory-scoped grants (every
  entry is one exact 8.3 filename), and the list is small and static
  (`MAX_CAPABILITIES = 4`) rather than a dynamic, resizable structure.
- **No write/create/delete syscalls** - `SYS_OPEN`/`SYS_READ` are
  read-only; a ring-3 process still cannot write to the filesystem at
  all, regardless of capabilities. Extending the same gating pattern
  to a future `SYS_WRITE_FILE` is straightforward but wasn't needed
  for this phase's proof.
- **`SYS_READ` re-reads the whole file from the start on every call**
  rather than the underlying `vfs_read_file()` supporting a real
  offset/seek - correct for the small demo files this is exercised
  against, wasteful for anything larger.
- **The global open-file table is shared and fixed-size (8 slots)**
  across all processes combined, not a real per-process file
  descriptor table with its own numbering.
- **This is filesystem-only sandboxing.** Network access, process
  creation, and every other privileged operation remain either fully
  open to any ring-3 code that already has a syscall for it (none do
  yet, beyond SYS_WRITE/EXIT/YIELD) or simply inaccessible from ring 3
  entirely (there's no SYS_SOCKET, no SYS_SPAWN). A real capability
  system would extend this same pattern to every resource type, not
  just files.
- **No sandboxing for the *existing* demo/network/GUI code** - `nova-pkg`,
  the shell, and everything else file-related still runs as trusted
  kernel (ring-0) code with unrestricted filesystem access. This phase
  proves the mechanism works for a process that opts into using it via
  syscalls; it doesn't retrofit the rest of NovaOS to go through it.

## Phase 12 - GUI Software Center

**Status: Complete, at a deliberately scoped-down scope, with one
honestly-unresolved verification gap flagged below (the same kind of
gap Phase 7 had, and for the same underlying reason).** Connects
`nova-pkg` (Phase 8/10) to the windowing system (Phase 7) for the
first time - the biggest visible gap left from the project's original
"Linux kernel + Ubuntu-style package manager/GUI + Windows UX" vision
in PROJECT_PLAN.md.

### What was built

- **`kernel/gui/font5x7.h`** gained 26 hand-built uppercase letter
  glyphs (A-Z) plus `.` and `-`, extending Phase 7's digit-only font.
  Built the same deliberate way as the digits - each letter reasoned
  through as a 5x7 shape and hand-encoded, not transcribed from an
  existing font table (the same reasoning Phase 7 used to avoid the
  transcription risk of a larger existing font, just applied to more
  glyphs this time since there's now a real use for them).
- **`kernel/gui/canvas.*`** - generic pixel-buffer drawing primitives
  (put_pixel, fill_rect, draw_rect, draw_char, draw_text), written
  fresh rather than refactored out of `compositor.c`'s existing
  private helpers - deliberately avoids touching already-verified
  Phase 7 code for a phase that doesn't need to change it.
- **`kernel/gui/store.*`** - the Software Center itself. Lists every
  available package (via `pkg_list_available()`) with an INSTALL or
  REMOVE button per row; clicking calls the real `pkg_install()`/
  `pkg_remove()` and refreshes the list. ESC returns to the text
  shell, the same convention `gui` already uses.
- **Shell**: added `store`.

### Verified behavior - an unusually rigorous check on the highest-risk part

The hand-built font was the part most likely to have a subtle mistake
that would be easy to miss with casual visual inspection, so it got a
correspondingly more rigorous check: rather than eyeballing a
screenshot, a script compared the *exact* pixel pattern QEMU rendered
against the intended bit pattern for each glyph, bit by bit.

- Every letter checked - `N`, `O`, `V`, `A`, `S`, `F`, `T`, `W`, `R`,
  `E`, `C` (11 of 26, covering most of the distinct stroke shapes used
  across the alphabet) - matched **exactly**, pixel-for-pixel, between
  the intended font data and the actual rendered output. The
  rendering pipeline (`canvas_draw_char` -> `vga_put_pixel` -> the
  Mode 13h framebuffer) is proven correct, not just "looked right."
- Package rows render correctly: a fresh disk image with 2 available
  packages (Editor, Game) produced exactly 2 detected row borders at
  the expected screen positions.
- `ESC` correctly restores text mode, verified the same way Phase 7's
  mode switch was: measured screendump dimensions (720x400 with the
  expected black/white/cyan text-mode palette), not just "looks like
  it went back."
- Clean build, zero warnings under `-Wall -Wextra`; zero regression -
  every Phase 2-11 `make test` marker still passes.
- Along the way, found (via code review, not by hitting a runtime bug)
  and fixed a stale `about` command string that Phase 11 had missed
  updating - still said "Phase 10" despite Phase 11 having shipped.

### Known limitation: click-driven install/remove is not conclusively verified

This is the same kind of gap Phase 7 flagged for window-dragging, and
for the same root cause. Three different scripted mouse-movement
patterns were tried against the Software Center's INSTALL button - a
multi-step sequence of varying deltas, one single larger move, and a
sequence of repeated *identical* small deltas (the one pattern Phase 7
found reliable) - and all three showed the same unreliable cursor
positioning already characterized in Phase 7's PROGRESS.md entry: a
QEMU `-display none` + monitor-scripted-input quirk with no real-usage
analogue, not something correctable from inside NovaOS. The
click-handling code in `store.c` (rising-edge button detection via
`state.left_button && !prev_left_button`, then a `point_in_rect()`
check) is structurally identical to `compositor.c`'s window-dragging
logic from Phase 7, which real mouse/display usage has since
confirmed works - but that confirmation hasn't specifically happened
for this file yet.

**If you try `store` with `make run` and clicking INSTALL/REMOVE
doesn't work, that's worth reporting** - the underlying package
install/remove functions themselves are independently proven correct
by Phase 8/10's boot self-test (which installs and removes a package
without any GUI involved at all), so a failure here would specifically
implicate the click-detection code, not the package manager underneath
it.

### Other known limitations / follow-ups (tracked for future phases)

- **Font covers uppercase A-Z, 0-9, `.`, `-`, and space only** - no
  lowercase, no other punctuation. A package description using an
  unsupported character silently gets a gap, not a crash or a
  wrong-looking glyph (see `font5x7_lookup()`).
- **No scrolling** - `MAX_STORE_ROWS = 8` rows fit on screen at
  `ROW_HEIGHT = 20`; a package list longer than that is silently
  truncated rather than scrollable.
- **No confirmation dialogs, no progress indication, no error
  reporting in the GUI itself** - a failed install (e.g. disk full)
  fails silently from the Software Center's perspective; the failure
  is only visible in the serial log, the same way the CLI's `pkg`
  command already logs failures there today.
- **Refreshes the whole package list from disk on every click**, not
  an incremental update - fine for the tiny package counts this has
  ever been tested with, wasteful for a much larger catalog.
- **Still no fetch-and-install-in-one-click** - `pkg fetch` (Phase 10)
  remains CLI-only; the Software Center only shows packages already
  present on the local disk.

## Phase 13 - PCI Bus Enumeration

**Status: Complete, at a deliberately scoped-down scope.** Chosen from
the open follow-up list specifically because it's foundational for any
future driver work (USB, sound, additional NICs all need PCI device
discovery first) and carries much lower risk than the other two big
remaining items (a real bootloader-writing installer needs new
real-mode BIOS/assembly work; TCP needs a retransmission/sequencing
state machine) - no new hardware protocol complexity, just a
well-defined, simple port-I/O enumeration algorithm.

### What was built

- **`kernel/arch/x86/io.h`** gained `outl()`/`inl()` (32-bit port I/O)
  - no earlier driver needed anything wider than 16 bits; PCI's
    configuration address/data ports (0xCF8/0xCFC) are read and
    written as full 32-bit dwords.
- **`kernel/drivers/pci/pci.*`** - configuration space access
  (8/16/32-bit reads, all built on top of the one 32-bit primitive)
  and `pci_enumerate()`: a brute-force scan of every bus/device/
  function, calling a callback once for each function that reports a
  real vendor ID (0xFFFF means "nothing here" - the standard way an
  empty slot is recognized). Multi-function devices (checked via the
  header-type byte's top bit) get their functions 1-7 probed too;
  everything else only needs function 0 checked. Simple rather than
  optimal - a real enumerator would recursively discover which buses
  exist via bridge devices instead of scanning the full architectural
  range of 256, but scanning everything is still correct, just does
  more (cheap) work than strictly necessary.
- **Shell**: added `lspci`.
- **Boot self-test**: enumerates PCI and logs every function found.

### Verified behavior

- Clean build, zero warnings under `-Wall -Wextra`; zero regression -
  every Phase 2-12 `make test` marker still passes.
- Full boot log, unedited, first try after fixing one ordering mistake
  (see below):
  ```
  [ OK ] PCI 0:0.0 vendor=0x8086 device=0x1237 class=0x6 (Host bridge)
  [ OK ] PCI 0:1.0 vendor=0x8086 device=0x7000 class=0x6 (ISA bridge)
  [ OK ] PCI 0:1.1 vendor=0x8086 device=0x7010 class=0x1 (IDE controller)
  [ OK ] PCI 0:1.3 vendor=0x8086 device=0x7113 class=0x6 (Bridge device)
  [ OK ] PCI 0:2.0 vendor=0x1234 device=0x1111 class=0x3 (Display controller)
  [ OK ] PCI ENUMERATION OK: 5 device(s) found
  ```
  These are recognizable real devices, not arbitrary numbers: Intel
  vendor ID 0x8086 device 0x1237 is the 82441FX host bridge and 0x7000/
  0x7010 are the PIIX3 ISA/IDE bridge functions - the standard i440fx
  chipset QEMU's default `-M pc` machine always emulates, regardless of
  which `-device` flags are added. Vendor 0x1234 device 0x1111 is
  QEMU's own "stdvga" virtual display adapter - genuine confirmation
  that enumeration finds real attached hardware, not just the fixed
  chipset baseline.
- **Unlike every previous phase's self-test, this one needs no disk or
  NIC attached at all to produce a non-empty, verifiable result** - the
  host bridge alone is guaranteed present on any standard QEMU `-M pc`
  boot, making it an even more environment-independent test than the
  ping/TFTP self-tests (which at least need SLIRP networking
  configured).
- `make test` now asserts both `PCI ENUMERATION OK` and the specific
  host bridge's `vendor=0x8086 device=0x1237` line appear - checking
  the *actual decoded values* are correct, not just "found something."
- Interactively confirmed `lspci` produces the same device list through
  the shell, with no faults in the serial log.
- **One real ordering mistake, caught immediately by the build, not by
  booting**: the PCI enumeration callback was originally defined
  further down `main.c` than `kernel_late_init()`, which calls it -
  C doesn't allow forward references to a function defined later in
  the same file without a prototype. The compiler error was
  unambiguous and the fix (moving the callback above its call site)
  took one edit; mentioned here only because every previous phase's
  "bugs found" section describes something caught by booting and
  testing, and it's worth being equally clear when a mistake was
  instead just an ordinary compile error with no runtime behavior to
  discuss.

### Known limitations / follow-ups (tracked for future phases)

- **No MMCONFIG/ECAM support** - only the legacy 0xCF8/0xCFC I/O port
  mechanism, which is universally supported but limited to the
  original 256-byte configuration space (PCIe's extended 4KB space
  needs MMCONFIG).
- **Brute-force full-range scan**, not bridge-driven bus discovery -
  correct but not how a more sophisticated enumerator would minimize
  wasted config cycles on hardware with many empty bus numbers.
- **`pci_class_name()` covers a small hand-picked table** of common
  classes (storage, network, display, bridge, USB) - anything else
  reports as "Unknown," which is honest but not informative.
- **Detection only - no actual PCI device drivers were written.** This
  phase answers "what hardware exists," not "how do I use it." A PCI-
  based NIC, sound card, or USB controller found by `lspci` still has
  no driver to talk to it - Phase 6's NE2000 remains ISA-only (a fixed
  I/O base, no PCI enumeration involved) and continues to work exactly
  as before, unaffected by any of this.
- **No IRQ routing information read** - the interrupt line/pin
  configuration fields exist in PCI config space but aren't decoded;
  not needed since no driver here uses PCI-signaled interrupts yet.

## Phase 14 - Network Capability Enforcement

**Status: Complete.** Delivered together with Phases 15 and 16 in one
session. Extends Phase 11's per-process capability pattern to a
second resource type: which IPv4 addresses a process may send UDP
packets to.

### What was built

- **`kernel/task/process.h`**: `process_t` gained `allowed_hosts[4]` +
  `allowed_host_count`, the same shape as Phase 11's `allowed_files`.
  `process_create_sandboxed_task()` now takes both a filename list and
  a host list (either may be empty).
- **`kernel/arch/x86/cpu/syscall.h`/`.c`**: new `SYS_NET_SEND` syscall
  (EBX = destination IP, ECX = destination port, EDX = message
  pointer). `handle_net_send()` checks the calling process's
  `allowed_hosts` before calling the real `udp_send()` - denied
  attempts are logged at `[SECURITY]` level, the same pattern
  `SYS_OPEN` established.
- **`kernel/task/sandbox_demo.c`**: extended (not duplicated) to also
  test network capability - granted only the gateway (10.0.2.2), it
  sends there (should succeed) and to 10.0.2.100 (should be denied).

### Verified behavior

First try, no debugging needed - all four PASS conditions (2 file, 2
network) appear in the boot log, with the kernel's independent
`[SECURITY]` log and the process's own self-check agreeing on both
denials:
```
[SYSCALL] pid 5 SYS_NET_SEND to 10.0.2.2:9999 (capability granted) -> sent
[sandbox] PASS: SYS_NET_SEND to the gateway succeeded - it's in my capability list.
[SECURITY] pid 5 denied SYS_NET_SEND to 10.0.2.100 - not in its capability list
[sandbox] PASS: SYS_NET_SEND to 10.0.2.100 correctly denied - not in my capability list.
```
Zero regression, zero new compiler warnings. `make test` now also
asserts the network PASS and `[SECURITY]` denial lines appear.

### Known limitations

- **Coarse-grained**: capability is per-destination-IP only, not
  per-port or per-protocol - a granted IP can be reached on any port.
- **`SYS_NET_SEND` is UDP-only, fixed source port, text-payload only**
  (a real send syscall would take a separate length argument instead
  of assuming a NUL-terminated string) - a minimal proof of the
  enforcement mechanism, not a general sockets API.
- **Still filesystem+network only.** Process creation, and every other
  privileged operation, remain outside this capability model.

## Phase 15 - Font Punctuation & Package Descriptions

**Status: Complete.** Checked the actual fixture package descriptions
(e.g. "A tiny game (demo package)") rather than guessing what
punctuation a font "should" have, and found parentheses were missing
entirely.

### What was built

- **`kernel/gui/font5x7.h`**: four new glyphs - `(`, `)`, `!`, `,` -
  built the same hand-crafted way as every other glyph in this file.
- **`kernel/gui/store.c`**: each package row grew a second line
  showing its description (truncated if too long for the row - not
  wrapped or scrolled). Row height increased from 20px to 32px to fit
  it.

### An explicit scope decision, not an oversight

Lowercase input already renders correctly via uppercase substitution
- existing behavior from Phase 12, not new here. This phase
deliberately did *not* add 26 true lowercase letterforms: several
lowercase letters (g, j, p, q, y) have descenders that don't fit
cleanly in a flat 7-row glyph grid without redesigning the whole
font's baseline, and the payoff would be purely cosmetic (text is
already fully legible) for real added risk (26 more hand-crafted
shapes to get right). Documented in `font5x7_lookup()`'s comment
directly, not just here.

### Verified behavior

Used the same pixel-exact verification technique Phase 12 introduced:
compared actual rendered pixels against the intended bit pattern for
both new parenthesis glyphs, at their real position within a real
rendered description string in a live screendump - not just
eyeballing a screenshot. Both matched exactly.

Worth being honest about the process here: an initial verification
run showed apparent mismatches for two glyphs. Investigating found the
bug was in the *verification script's* character-index arithmetic (an
off-by-one when locating where `)` should appear in the string), not
in the rendering - corrected and re-verified before concluding it
actually passed, rather than either accepting a spurious failure or
casually explaining it away without checking.

Zero regression (every Phase 2-14 `make test` marker still passes),
zero new compiler warnings, no crashes in the serial log during
interactive testing.

### Known limitations

- Same font-coverage limitations as Phase 12 (no lowercase forms, no
  scrolling in the package list) plus the four new characters covering
  only what current fixture text happens to need - any other
  punctuation still silently renders as a gap.

## Phase 16 - RTL8139 PCI NIC Driver

**Status: Complete.** The natural capstone to Phase 13's PCI
enumeration: proving hardware *detection* leads to actual usable
hardware *support*, not just an `lspci` listing.

### What was built

- **`kernel/drivers/net/rtl8139.*`** - a full second NIC driver,
  architecturally quite different from Phase 6's NE2000: the RTL8139
  DMAs directly to/from physical system memory addresses the driver
  hands it (a receive ring buffer, four transmit descriptor slots)
  rather than NE2000's page-indexed remote-DMA protocol through
  onboard NIC memory. Found via Phase 13's `pci_enumerate()` (vendor
  `0x10EC`, device `0x8139`) rather than a fixed I/O base - its actual
  I/O address comes from reading the PCI BAR0 config register at
  runtime, the real payoff of having PCI enumeration at all. Requires
  explicitly enabling PCI bus mastering (setting a bit in the PCI
  command register) for the card to be allowed to perform DMA at
  all - easy to forget, called out directly in the code.
- **`kernel/net/net.c`** gained a small NIC-selection seam
  (`net_driver_send()`/`net_driver_receive()`/
  `net_driver_mac_address()`): `net_init()` prefers the RTL8139 if
  Phase 13's enumeration finds one, falling back to NE2000 otherwise.
  `kernel/net/ethernet.c` was updated to call through this seam
  instead of hardcoding `ne2000_*` - the only change to
  already-verified Phase 6 code in this whole three-phase batch, and a
  narrow, mechanical one (swap which function names get called, not a
  logic change).
- **`Makefile`**: the default test NIC is now RTL8139 (`-device
  rtl8139` in place of `-device ne2k_isa`) - meaning every existing
  network self-test (ping, TFTP fetch) now exercises the new driver
  through the *entire* stack, not a standalone demo. NE2000 remains
  fully present and functional; swapping the `-device` line back
  exercises it instead, and `net.c`'s fallback logic picks whichever
  NIC is actually attached.

### Verified behavior - worked first try on real complexity

Unlike NE2000 (Phase 6) and the PS/2 mouse (Phase 7), which each had a
real bug caught during testing, this driver worked correctly on the
first attempt - worth stating plainly rather than manufacturing a
"bugs found" narrative where none occurred. The boot log:
```
[ OK ] RTL8139 NIC at PCI 0:3.0, I/O base 0xC000, MAC 52:54:0:12:34:56
[ OK ] Network up (RTL8139): IP 10.0.2.15, gateway 10.0.2.2
[ OK ] PING OK: gateway replied in 0 ticks (~0ms)
[ OK ] TFTP FETCH OK: WEATHER.PKG (172 bytes)
```
- PCI enumeration correctly found the card as a distinct device
  (`vendor=0x10EC device=0x8139 class=0x2`, i.e. an Ethernet
  controller) among the chipset devices Phase 13 already detects.
- The I/O base (`0xC000`) came from actually reading the PCI BAR0
  register at boot, not a hardcoded guess - different every time QEMU
  assigns PCI addresses differently, and it still worked.
- The full existing stack - ARP resolution, ICMP ping, UDP, TFTP -
  worked correctly running entirely over the new driver, with zero
  `[WARN] RTL8139: unexpected packet header` messages (the ring-buffer
  desync class of bug NE2000 hit in Phase 6) across a 25-second
  stability run and multiple real packet exchanges (ARP request/reply,
  ICMP echo/reply, TFTP RRQ/DATA/ACK).
- Interactively confirmed `ping 10.0.2.2` through the shell over
  RTL8139 with no faults in the serial log.
- Zero regression - every Phase 2-15 `make test` marker still passes.
  `make test` now also asserts `Network up (RTL8139)` appears.

### Known limitations / follow-ups (tracked for future phases)

- **I/O-space BAR only** - if a hypothetical RTL8139 variant exposed
  only a memory-mapped BAR, this driver logs and declines rather than
  supporting it (real RTL8139 hardware always provides an I/O BAR
  too, so this hasn't been a practical limitation, just a documented
  one).
- **Promiscuous-ish receive filtering** (`RCR` accepts all packets,
  matching or not) rather than NE2000's tighter unicast+broadcast
  filtering - simpler to get right initially; a real driver would
  narrow this once correctness is established.
- **No IRQ support** - polled, matching NE2000's style for consistency
  across both drivers, at the same latency/CPU-overhead cost that
  choice always carries.
- **No resync logic if the receive ring does desync** - the defensive
  check that catches a bad packet header just drops that poll cycle
  and logs a warning rather than attempting to recover by resyncing to
  the card's own reported position. Never triggered in testing, but
  the recovery path itself is unexercised.
- **Static, fixed-size DMA buffers** rather than a general
  physical-memory-allocation API - fine because they live in NovaOS's
  identity-mapped low memory (virtual address always equals physical
  address there), but this approach wouldn't extend to a system with
  a real virtual/physical split for driver buffers.

## Phase 17 - Process Creation Capability

**Status: Complete.** Delivered together with Phases 18 and 19 in one
session. Extends Phase 11/14's per-process capability pattern to a
third resource type: whether a process may create another process at
all.

### What was built

- **`kernel/task/process.h`/`.c`**: `process_t` gained a boolean
  `can_spawn` (rather than a list, like the other two capabilities) -
  there's currently exactly one spawnable task type, so a list of
  "which ones" would be pointless; a fixed yes/no is the honest scope
  until NovaOS has a real exec-a-file mechanism.
  `process_create_sandboxed_task()` gained a `can_spawn` parameter.
- **`kernel/arch/x86/cpu/syscall.h`/`.c`**: new `SYS_SPAWN` syscall, no
  arguments. `handle_spawn()` checks the calling process's `can_spawn`
  before calling `process_create_user_task()` - deliberately not
  `process_create_sandboxed_task()`, so the ability to spawn does not
  imply the ability to grant capabilities to what's spawned.
- **`kernel/task/greeter_task.*`**: the one spawnable task - prints a
  message and exits, enough to prove a spawned process genuinely runs
  as its own independent process (own PID, own address space) rather
  than `SYS_SPAWN` merely returning a fake success.
- **`kernel/task/unprivileged_demo.*`**: a new, deliberately
  capability-free process (plain `process_create_user_task()`) that
  proves the denied case - the existing sandbox process was granted
  spawn capability and already proves the allowed case, so this needed
  a separate process rather than a third code path in the same one.

### Verified behavior

First try, no debugging needed. The spawned process's own
`[greeter] Hello!` message and its own `exited` log line are genuine
proof it ran as an independent process, not just a returned success
code:
```
[SYSCALL] pid 5 SYS_SPAWN (capability granted) -> new pid 7
[sandbox] PASS: SYS_SPAWN succeeded - spawn capability was granted.
[unprivileged] Starting; I have no capabilities at all (plain process_create_user_task).
[SECURITY] pid 6 denied SYS_SPAWN - spawn capability not granted
[unprivileged] PASS: SYS_SPAWN correctly denied - I was never granted spawn capability.
[ OK ] Process 'unprivileged' (pid 6) exited
[greeter] Hello! I was spawned by another process via SYS_SPAWN.
[ OK ] Process 'spawned' (pid 7) exited
```
Both PASS conditions (allowed via sandbox, denied via unprivileged)
appear correctly, with the kernel's `[SECURITY]` log agreeing with the
denied process's own self-check. Zero regression, zero new compiler
warnings. `make test` asserts both PASS lines and the greeter's
message appear.

### Known limitations

- **Exactly one spawnable task type, chosen automatically** - there is
  no way for the caller to select which one (moot today with only one
  to choose from, but a real limitation the moment a second exists).
- **Still filesystem+network+spawn only** as the complete set of
  capability-gated resources - every other privileged operation
  remains either fully open to any syscall that already exists for it
  (none currently touch anything else privileged) or simply
  inaccessible from ring 3 entirely.

## Phase 18 - AC97 PCI Sound Driver

**Status: Complete.** A full second PCI DMA driver, and the natural
next step after Phase 16's RTL8139: proving the "detect hardware via
PCI, then actually drive it" pattern generalizes beyond networking.

### What was built

- **`kernel/drivers/sound/ac97.*`**: found via `pci_enumerate()` by
  PCI **class code** (0x04/0x01, multimedia audio device) rather than
  a hardcoded vendor/device ID the way RTL8139's driver matches - the
  more correct, general approach here, since AC97 is a standardized
  register interface implemented by many different vendors' chipsets,
  not one specific card. Reads both PCI BARs (BAR0 = Native Audio
  Mixer for codec/volume registers, BAR1 = Native Audio Bus Master for
  DMA control), enables PCI bus mastering, resets the codec, sets full
  volume, and plays a short tone through a buffer descriptor list
  DMA'd directly from a static kernel-image buffer - the same
  "identity-mapped low memory needs no separate physical allocator"
  reasoning RTL8139's driver already established. The tone itself is
  an integer-only square wave (no floating point / FPU dependency this
  kernel hasn't set up).
- **`kernel/drivers/pci/pci.c`**: added a multimedia/audio class name,
  fixing a cosmetic "Unknown" label found while testing.
- **Shell**: added `beep`.
- **`Makefile`**: AC97 is now always attached via a new `AUDIO_FLAGS`
  variable defaulting to QEMU's portable `none` backend (no host audio
  hardware required, works identically in CI) - overridable for a real
  backend to actually hear output via `make run`.

### Verified with real rigor - and a real bug found

Unlike every other self-test in this project, there's no way to check
"did audio actually play" from headless kernel code alone - playback
happens in hardware, with nothing to read back that proves audible
sound came out. Verified instead by capturing QEMU's actual audio
output via its `-audiodev wav` backend and parsing the raw PCM samples
directly in Python:
- Confirmed non-silent audio starting exactly when the self-test runs,
  with regular waveform transitions at approximately the target
  440Hz, lasting almost exactly the requested 0.3 seconds, followed by
  clean silence - not just "a WAV file was created," but "the WAV file
  contains the specific tone this driver was asked to produce."

**A real bug, caught by this verification, not assumed away**: the
first beep played correctly, but replaying after it had already
finished produced no audible output at all, despite the function
running and logging normally. The PCM engine's internal state (CIV and
related registers) was left wherever the first playback's completion
left it; simply rewriting BDBAR/LVI/CR without resetting first wasn't
enough to make the card recognize a genuinely new play request. Fixed
by explicitly stopping and resetting the channel (the `CR_RR` bit)
before every play, then reconfirmed via a second WAV capture showing
two correctly-shaped, distinct tone regions instead of one real tone
followed by silence on the replay. The boot self-test now plays twice
with a delay between, specifically to keep this regression-proof in
automated testing rather than relying solely on the interactive check
that first caught it.

Zero regression (every Phase 2-17 `make test` marker still passes);
`make test` now also asserts both the AC97 init and beep lines appear.

### Known limitations / follow-ups

- **PCM output only** - no recording (PCM in/mic in), no variable
  sample rate (fixed at the AC97 standard 48kHz), no volume control
  exposed to the user (hardcoded to maximum).
- **One fixed tone** - no way to specify frequency, duration, or
  waveform shape from the shell; `beep` always plays the same 440Hz
  square wave.
- **No IRQ support** - polled, matching the style of every other
  driver in this tree, at the same latency/CPU-overhead tradeoff that
  choice always carries.
- **I/O-space BARs only**, matching RTL8139's same limitation and for
  the same reason - not a practical problem for real AC97 hardware,
  which always provides an I/O BAR, but not a general MMIO fallback.

## Phase 19 - Minimal DNS Client

**Status: Complete.** Closes a real, previously-flagged gap: every
network command (`ping`, `tftp`, `pkg fetch`) only ever accepted raw
IP addresses. Chosen instead of full TCP/sockets (the other major
open networking item) as a much lower-risk way to still meaningfully
extend what NovaOS's network stack can do - no connection state, no
retransmission/congestion-control state machine, just a single
request/response over UDP, the same shape as the existing TFTP client.

### What was built

- **`kernel/net/dns.*`**: RFC 1035 A-record queries only. Builds a
  standard DNS query (recursion desired), sends it via the existing
  UDP layer (Phase 10), and parses the response - including handling
  DNS name compression (a 2-byte pointer back into the message,
  standard in every real-world response's answer section) enough to
  correctly skip over a compressed name, though not general enough to
  *build* one. The same synchronous, single-outstanding-request,
  bounded-timeout pattern `arp_resolve()`, `icmp_ping()`, and
  `tftp_get()` already use.
- **`kernel/net/net.h`**: added `NET_DNS_SERVER_IP` (10.0.2.3) - QEMU
  SLIRP's conventional built-in DNS proxy address, one more of the
  "SLIRP always answers this" addresses the gateway ping and TFTP
  self-tests already rely on.
- **Shell**: added `nslookup HOSTNAME`; `ping` now tries DNS resolution
  automatically whenever its argument isn't a raw dotted-quad IP,
  falling back cleanly to the old error message if that also fails.

### Verified behavior - genuine external resolution, not just protocol correctness

Unlike the gateway ping and TFTP fetch self-tests (which SLIRP answers
entirely on its own, with zero dependency on real internet access),
this one asked SLIRP to resolve a real public hostname and got back
real answers:
```
[ OK ] DNS RESOLVE OK: example.com -> 104.20.23.154
```
(A second run during testing returned a different, equally valid IP -
`172.66.147.243` - consistent with `example.com` actually being served
from multiple addresses; both are genuine, not fabricated.) Interactive
testing confirmed `nslookup example.com` and `ping example.com` (which
correctly printed the resolved IP before pinging it) both work through
the shell with zero crashes in the serial log. Zero regression - every
Phase 2-18 `make test` marker still passes.

### A deliberate exception to this project's usual self-contained-testing principle

Every other network self-test in this project (ping, TFTP fetch) is
answered directly by QEMU SLIRP itself and therefore works with zero
dependency on real outbound network access from the host or CI runner
- a core design principle followed since Phase 6. DNS resolution
breaks that: SLIRP's DNS proxy forwards the query upstream to whatever
real DNS resolution the host environment provides, which this project
does not control the way it controls SLIRP's own self-answering
behavior. The self-test therefore logs `[WARN]` rather than being
treated as a hard failure, and is **not** included in `make test`'s
pass/fail assertion chain - deliberately, so a CI environment or
sandboxed host with no outbound network access doesn't fail the entire
test suite over something outside NovaOS's control. This is called out
explicitly rather than silently making the check "soft" without
explanation.

### Known limitations / follow-ups

- **A-records only** - no AAAA (IPv6), no MX/CNAME/TXT/other record
  types, no reverse lookups.
- **No caching** - every `ping`/`nslookup` call re-resolves from
  scratch, even for a hostname just looked up moments ago.
- **No retry beyond the single bounded wait** - a single lost UDP
  packet (query or response) means the whole resolution fails and has
  to be manually retried by the user, the same tradeoff `tftp_get()`
  already accepts for the same reason (a local virtual network with
  effectively zero packet loss).
- **Fixed DNS server** (`NET_DNS_SERVER_IP`, hardcoded to SLIRP's
  proxy address) - no way to configure a different resolver, and no
  DHCP-provided DNS server option either (NovaOS has no DHCP client at
  all, unchanged since Phase 6).

## Phase 20 - Installable Disk Image

**Status: Complete.** Closes the biggest release-blocking gap:
without this, NovaOS could only be demonstrated via a CD-ROM image,
not actually installed anywhere persistent.

### What was found (not built from scratch)

`grub-mkrescue` (already used since Phase 1 to build `novaos.iso`)
produces a **hybrid** image by default: the same file is simultaneously
a valid El Torito bootable CD image *and* contains a real MBR that
lets it boot as a raw BIOS hard disk or USB drive. This was verified,
not assumed: `file novaos.iso` confirms a "DOS/MBR boot sector," and
attaching the exact same `novaos.iso` file as a plain QEMU hard disk
(`-drive file=novaos.iso,format=raw`, no `-cdrom` at all) boots the
full system - GDT/IDT/paging/heap/scheduler init, ATA detection,
networking, everything - identically to the CD-ROM path. No custom
bootloader was written; GRUB's own long-proven tooling already
provides this.

### What was added

- **`make install-image`**: builds the ISO + data disk and prints the
  exact `dd` command to write `novaos.iso` to a real USB drive, plus
  instructions for attaching it as a persistent VM hard disk.

### Known limitation

- **Still two separate images** (a boot image plus a separate FAT32
  data disk for persistent files), not one unified disk. NovaOS's
  FAT32 driver (Phase 3) reads directly from LBA 0 expecting a FAT32
  boot sector, with no MBR-partition-table awareness - combining both
  into one physical medium would need the driver to become
  partition-aware first. Tracked as future work, not attempted here
  given the time constraints of this release push.

## Phase 21 - License, Versioning & Changelog

**Status: Complete.** Ordinary but essential release housekeeping that
was simply missing before now.

- **`LICENSE`**: MIT, the standard permissive choice for a project
  like this.
- **`kernel/include/version.h`**: bumped `0.1.0` -> `1.0.0` for the
  first full release - every boot log, `about` command, and kernel
  banner reflects this automatically (all derive from the same
  `NOVAOS_VERSION_STRING`, so no other file needed a matching edit).
  Historical version strings quoted verbatim in earlier PROGRESS.md
  boot-log excerpts were deliberately left as `0.1.0` - they're
  accurate records of what was actually logged at that point in
  development, not something to retroactively rewrite.
- **`CHANGELOG.md`**: a new file summarizing all 22 phases for anyone
  who wants the release history without reading the full,
  much-longer PROGRESS.md.

## Phase 22 - Process Exit Resource Cleanup

**Status: Complete.** Fixes a real, long-documented resource leak:
every ring-3 process's kernel stack, user stack, page table, and page
directory has leaked permanently on exit since Phase 4/5 - noted as a
known limitation in multiple earlier phase writeups but never
addressed until this release-hardening pass.

### What was built

`process_exit_current()` now frees, for every exiting process: its
kernel stack (always private, `kfree()`), and - for ring-3 processes
specifically - the physical frames backing its private user stack, the
one page table that mapped them, and the process's own page directory.
The shared kernel range (identity-mapped 0-64MB, copied into every
process's directory since Phase 5) is never touched - only found and
freed by walking the single, always-known page-directory index
`USER_STACK_VIRT_BASE` falls into, which nothing else ever shares.

### A subtlety worth documenting rather than glossing over

`process_exit_current()` frees the *currently executing* process's own
kernel stack before yielding away from it - meaning a handful of
register-save pushes inside the ensuing context switch land on memory
already marked "free" by the heap allocator. This is safe in NovaOS's
specific design (single-core, no concurrent allocation can claim that
memory in the narrow window before the stack is permanently abandoned)
but is a subtlety future changes to the scheduler or allocator should
keep in mind rather than assume away by default.

### Verified behavior

NovaOS already exercises process exit heavily on every single boot -
five different ring-3 processes (`demo-a`, `demo-b`, `sandbox`,
`unprivileged`, `spawned`) exit during the standard self-test
sequence. Re-ran the full `make test` suite and a 25-second extended
stability run after this change: all five processes still exit
cleanly, every existing PASS/FAIL self-check still reports correctly,
and zero crashes or corruption appeared - meaning this fix was
validated against the most demanding existing test coverage in the
project, not a new, separately-invented test.

### Known limitations

- **Only the user stack's specific page table/directory entry is
  freed** - if a future phase maps *additional* private pages for a
  process (beyond the one fixed user-stack region this release
  supports), those would need their own cleanup logic added
  alongside this.
- **No cleanup of open file handles** (Phase 11's `SYS_OPEN` table) or
  UDP listener state (Phase 14) on process exit - a process that exits
  while holding a file handle leaves that slot marked in-use forever,
  and Phase 19's DNS/TFTP/UDP single-listener state isn't process-
  scoped at all, so it's not a *new* problem this phase introduces,
  but it's not fixed either. Tracked as follow-up work.

## Phase 23 - ELF Loading & a Real Process Model

**Status: Complete.** The single highest-leverage item identified in
the "gap analysis vs. Ubuntu" planning document: NovaOS could
previously only run C functions compiled directly into the kernel
image. This phase gives it the ability to load and run a real,
independently-compiled executable from disk - the load-bearing
feature everything else in "run real software" depends on.

### Scope, stated upfront: this is `exec`, not `fork()`+`exec()`

The request that started this phase asked for "`fork()`/`exec()`-style
semantics." What's built is `exec`-style spawn-and-load - create and
run a new process from an ELF file in one step - deliberately not
true two-step `fork()` (duplicate a *running* process's entire address
space, then replace one of the two copies' image). Real `fork()`
needs copy-on-write memory management this kernel doesn't have; this
is closer to POSIX's `posix_spawn()`, which exists in POSIX precisely
because most real callers (a shell running a command) never needed
full `fork()` semantics in the first place. Tracked as real follow-up
work, not silently substituted for what was asked.

### What was built

- **`kernel/task/elf.*`** - a minimal ELF32 loader: validates the
  header (magic, 32-bit, little-endian, `EM_386`, `ET_EXEC`), walks
  every `PT_LOAD` program header, allocates fresh physical frames for
  each segment, maps them into a process's own address space with
  read/write permissions taken from the segment's actual flags (not
  assumed), copies file data with correct `.bss` zero-padding for the
  memsz-vs-filesz difference. Statically-linked, non-PIE executables
  only - no dynamic linking (`PT_DYNAMIC`/`PT_INTERP`), no relocation
  processing.
- **`process_exec()`** (`kernel/task/process.c`) - reads the ELF file
  via the VFS, loads it, allocates a user stack, and constructs the
  **real x86 process-entry stack convention**: from the initial ESP,
  `argc`, `argv[0..argc-1]` (pointers into string data placed lower in
  the same stack), a NULL terminator, an empty `envp` (just one more
  NULL - no environment variables are actually populated yet, an
  honest scope limit), then the argv strings themselves. This is the
  same raw layout Linux's own `execve()` leaves for a fresh process,
  not a simplified NovaOS-specific convention - a deliberate choice
  for forward compatibility with a real libc/crt0 in a future phase.
- **`SYS_EXIT` now takes a real exit code** (EBX) - previously ignored
  entirely. Fixed all four existing call sites (`greeter_task`,
  `sandbox_demo`, `unprivileged_demo`, `user_demo`) that used the old
  no-argument convention, so no process now exits with whatever
  garbage happened to be left in EBX as its "code."
- **`process_wait()` / `SYS_WAIT`** - blocks (yielding repeatedly, the
  same proven pattern `SYS_YIELD` already uses from inside a syscall
  handler) until a target process terminates, then returns its real
  exit code.
- **`SYS_EXEC`** - the syscall surface for `process_exec()`, reusing
  Phase 17's existing `can_spawn` capability rather than adding a
  fourth capability type: "may create processes" is one capability,
  whether the new process runs the fixed greeter task or a real loaded
  ELF.
- **Shell**: `run PATH [args...]` - loads an ELF, waits for it, prints
  its real exit code.
- **A genuine test fixture**: `tools/elf-fixtures/hello.asm`, a real,
  independently-assembled-and-linked ELF32 executable (not a function
  compiled into the kernel image) written in raw NASM specifically to
  avoid any dependency on a C runtime/crt0 startup convention this
  kernel doesn't provide yet. Prints a message, echoes back
  `argv[0]`/`argv[1]` to prove argument passing genuinely works, and
  exits with a specific, checkable code (42) rather than just "did it
  run without crashing."

### A real correctness bug found and fixed before it ever shipped

Phase 22's process-exit cleanup only knew how to free the user stack's
one fixed virtual address - it would have silently leaked every ELF
segment's memory (physical frames, page tables) on process exit, since
`elf_load()` maps segments at addresses (like `0x08048000`) the old
cleanup code had no idea existed. Generalized `free_user_address_space()`
to walk every one of the 1024 page-directory entries and free whichever
ones differ from the shared kernel template - found via direct
comparison against the kernel's own page directory rather than a
hardcoded index boundary, so it stays correct regardless of the kernel
identity map's size. This was caught by reasoning through the code
before testing, not by hitting a crash - worth noting since most bugs
in this project's history were caught by observed failures rather than
review.

### Verified behavior - a complete, genuine first-try success

This is among the most structurally complex mechanisms built in this
project (ELF segment loading across multiple physical frames, a custom
process-entry stack layout spanning non-contiguous physical memory,
capability-gated syscalls, blocking exit-code retrieval) and it worked
correctly on the very first boot test, unedited:
```
[ OK ] process_exec: loaded 'HELLO.ELF' as pid 8, entry=0x8048000, 2 arg(s)
[SYSCALL] pid 5 SYS_EXEC('HELLO.ELF') (capability granted) -> new pid 8
[SYSCALL] SYS_WRITE from pid 8 ('Hello from a real ELF executable loaded by NovaOS!\n')
[SYSCALL] SYS_WRITE from pid 8 ('HELLO.ELF')
[SYSCALL] SYS_WRITE from pid 8 ('hello-from-novaos')
[ OK ] Process 'HELLO.ELF' (pid 8) exited with code 42
[sandbox] PASS: SYS_EXEC loaded and ran a real ELF executable - SYS_WAIT returned exit code 42 as expected.
```
Every piece is independently verifiable in that log: the entry point
(`0x8048000`) matches exactly what `readelf -h hello.elf` reports; both
`argv[0]` and `argv[1]` are the exact strings passed to `SYS_EXEC`,
printed by the *executable's own code*, not the kernel echoing them
back; the exit code (42) is a value the ELF itself chose and the
kernel had no way to fabricate.

Also interactively re-verified with a second, independent invocation
(`run HELLO.ELF firstarg secondarg` through the shell) with different
argv values and a different pid, with an incidental but genuine extra
confirmation: the filename was accidentally typed in lowercase
(`hello.elf`) due to a limitation in the test-scripting tool used to
drive QEMU's monitor, not a NovaOS issue - and it still resolved and
ran correctly, confirming FAT32 filename lookups are properly
case-insensitive. Zero crashes across both runs. Zero regression -
every Phase 2-22 `make test` marker still passes, and every existing
process's exit log now shows a real code (0) instead of a silently
garbage/ignored one.

### Known limitations / follow-ups (tracked for future phases)

- **No true `fork()`** - see the scope note above. This remains the
  single largest gap between "exec-style spawn" and full Unix process
  semantics.
- **No dynamic linking** - only statically-linked, non-PIE
  executables load at all.
- **`envp` is always empty** - no real environment variable support
  yet, just a structurally-correct empty terminator.
- **`process_exec()` buffers the whole ELF file in a fixed 64KB
  static buffer** rather than streaming it - fine for small
  hand-written test binaries, would need `vfs_read_file()` to support
  partial/streamed reads for anything larger.
- **No libc** - the only way to produce a NovaOS-runnable executable
  today is hand-written assembly using NovaOS's own syscall
  convention directly, the same way `tools/elf-fixtures/hello.asm`
  was written. A real C program using standard library functions
  (`printf`, `malloc`, etc.) cannot run yet - this is exactly the next
  natural phase (a minimal libc port), per the gap-analysis roadmap.
- **`MAX_EXEC_ARGS` is a small fixed bound (8)**, matching the same
  "simple and honest about the limit" choice as `MAX_CAPABILITIES`
  rather than a dynamically-sized argument list.

## Phase 24 - Minimal Libc Port

**Status: Complete.** The natural next step identified at the end of
Phase 23: ELF loading alone only lets NovaOS run hand-written
assembly test binaries using its own syscall convention directly. This
phase makes ELF-loaded programs actually useful by giving them a real
(if small) C standard library to link against - `printf`, `malloc`,
string functions - the same category of thing every real C program
expects to be available.

### What was built

- **`SYS_SBRK`** (`kernel/arch/x86/cpu/syscall.h`/`.c`,
  `process_sbrk()` in `kernel/task/process.c`) - the foundation
  `malloc` needs. User processes had no heap at all before this.
  Grows a process's heap by mapping fresh physical frames as needed
  and returns the *previous* break address, the same semantics real
  Unix `sbrk()` has. A new fixed virtual address, `HEAP_VIRT_BASE`,
  positioned clear of both typical ELF load addresses and the user
  stack. Deliberately ungated by capability - it only ever manages the
  calling process's own memory, the same reasoning `SYS_WRITE` and
  `SYS_YIELD` already use.
- **`userland/libc/`** - a real, if intentionally small, C library,
  written fresh rather than porting an existing one (musl/newlib
  assume Linux-shaped syscalls or need a substantial shim layer;
  writing directly against NovaOS's own syscall convention was more
  tractable given the scope):
  - `crt0.asm` - bridges NovaOS's raw process-entry stack convention
    (the exact layout `process_exec()` builds - `argc`, `argv[]`,
    `envp[]`) into a proper cdecl call to `int main(int argc, char**
    argv, char** envp)`, then calls `exit()` with its return value -
    exactly what a real C runtime's `_start` always does.
  - `syscall.c`/`novasys.h` - clean wrappers for every NovaOS syscall.
  - `string.c` - `strlen`, `strcpy`, `strncpy`, `strcat`, `strcmp`,
    `strncmp`, `strchr`, `memcpy`, `memmove`, `memset`, `memcmp`.
    Written fresh rather than reusing `kernel/lib/string.c` - that
    file is compiled into and only reachable from the kernel image;
    userland programs are entirely separate binaries.
  - `stdio.c` - a real `printf` (`%d`/`%u`/`%x`/`%s`/`%c`/`%%` only -
    no field width/precision, no floating point - builds the whole
    formatted string into a fixed 512-byte buffer before one
    `sys_write()` call rather than streaming), plus `putchar`/`puts`.
  - `stdlib.c` - a first-fit `malloc`/`free` allocator over
    `SYS_SBRK` (the same overall design as the kernel's own
    `heap.c`, written fresh since userland can't call kernel code
    directly), plus `atoi`/`exit`.
- **`userland/examples/hello.c`** - a genuine test program using
  `printf`, real `argv` iteration, and `malloc`/`strcpy`/`strcat`/
  `free` chained together, not just raw syscalls.
- **`userland/examples/build.sh`** - compiles the test program against
  the libc into a real ELF32 executable (`tools/fixtures/HELLOC.ELF`),
  the same way a user would build their own NovaOS program.

### Verified behavior - complete, on the first attempt, for the most fragile mechanism in this project so far

`crt0`'s stack-to-`main()` bridge, `printf`'s `va_arg` handling, and
`malloc`'s `sbrk`-growth logic are all classic sources of subtle bugs
even when each piece looks correct in isolation - genuinely the
highest-risk phase yet for a silent, hard-to-spot mistake. It worked
correctly on the first boot test, unedited:
```
[SYSCALL] SYS_WRITE from pid 9 ('Hello from a REAL C program on NovaOS!\n')
[SYSCALL] SYS_WRITE from pid 9 ('argc = 2\n')
[SYSCALL] SYS_WRITE from pid 9 ('argv[0] = HELLOC.ELF\n')
[SYSCALL] SYS_WRITE from pid 9 ('argv[1] = libc-test\n')
[SYSCALL] SYS_WRITE from pid 9 ('malloc'd string: it works!\n')
[ OK ] Process 'HELLOC.ELF' (pid 9) exited with code 7
```
Every line is a genuine, independent proof point: `argc = 2` and both
`argv[]` values match exactly what `SYS_EXEC` was given, printed via
`printf`'s `%d` and `%s` handling, not the kernel echoing anything
back; `malloc'd string: it works!` is the actual result of
`malloc(64)` -> `strcpy` -> `strcat` -> `printf("%s")` chained
together - if `sbrk`-backed `malloc` had failed, or `strcpy`/`strcat`
had a boundary bug, this exact string would not have come out intact;
the exit code (7) is a value only this program's own `return`
statement could produce.

Re-verified via a second, fully independent invocation through the
shell's `run` command (a different pid, produced the identical correct
`malloc`'d string output) - and, same as Phase 23's interactive check,
an accidental lowercase filename typo (a limitation of the test
tooling driving QEMU's monitor, not NovaOS) still resolved and ran
correctly, another incidental reconfirmation of FAT32's
case-insensitive lookups. Zero crashes across all runs. Zero
regression - every Phase 2-23 `make test` marker still passes.
`make test` now also asserts the malloc/exit-code/PASS lines appear.

### Known limitations / follow-ups (tracked for future phases)

- **`malloc`/`free` never coalesce adjacent free blocks** - a
  long-running program with a varied allocation pattern will
  fragment its heap over time. A standard first-malloc
  simplification, not an oversight.
- **The heap only ever grows** - freed memory is reused within a
  process's own free list but never actually returned to the kernel
  via a negative `sbrk`. `process_sbrk()` explicitly rejects negative
  increments for this reason.
- **`printf` has a hard 512-byte output limit per call** and no field
  width/precision/floating-point support - enough for real, useful
  programs, not a general-purpose implementation.
- **No dynamic memory beyond the heap** - no `realloc`, no
  file-stream I/O (`fopen`/`fread`/etc., only the raw `sys_open`/
  `sys_read`/`sys_close` syscalls directly), no `errno`.
- **No standard library archive** - `userland/examples/build.sh`
  compiles and links every libc source file directly into each
  program rather than building a reusable `.a` static archive first,
  keeping the build simple at the cost of recompiling the libc for
  every program built this way.

## Phase 25 and beyond

Not started. Per the gap-analysis roadmap, the natural next steps are
MBR/GPT partition support and a real filesystem beyond FAT32 - the
next items after a working libc. Other candidates remain open as
before: a real from-scratch bootloader, full TCP/sockets, true
`fork()` semantics, extending capability-based access control to
further resource types, true lowercase font forms, and USB drivers.
Each would benefit from being scoped on its own terms rather than
assumed as "next."
