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
| P6-P9 | Not started |

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

## Phase 6 and beyond

Not started. See PROJECT_PLAN.md section 6 for scope.
