# NovaOS

[![Build & Boot Test](https://github.com/firoze-hossain/novaos/actions/workflows/ci.yml/badge.svg)](https://github.com/firoze-hossain/novaos/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
![Version](https://img.shields.io/badge/version-1.0.0-blue.svg)

A from-scratch x86 operating system in C and Assembly, combining a
Linux-style modular kernel, an Ubuntu-style friendly userland/package
manager, and Windows-style usability conventions. See
[PROJECT_PLAN.md](PROJECT_PLAN.md) for the full vision and roadmap,
[PROGRESS.md](PROGRESS.md) for exactly what's implemented today, and
[CHANGELOG.md](CHANGELOG.md) for the release history. Licensed under
[MIT](LICENSE).

## Installing NovaOS

`novaos.iso` is a hybrid image: the same file boots as a CD *and* as a
real BIOS hard disk/USB image - this is NovaOS's installer, and it
works today with no extra steps. Run `make install-image` for the
exact commands to write it to a USB drive or attach it persistently in
a VM.


## Features

**Phase 1 - Bootloader & Kernel Foundation**
- Multiboot-compliant kernel, loaded via GRUB
- VGA text-mode driver with 16-color support
- Freestanding C library subset (`printf`-family, `string.h` subset)
- Bootable ISO image, runs under QEMU

**Phase 2 - Memory Management & Interrupts**
- GDT with ring 0 / ring 3 segments
- Full IDT: 32 CPU exception handlers + 16 hardware IRQs (PIC remapped,
  masked-by-default)
- PIT timer driver (100 Hz) and PS/2 keyboard driver
- `kmalloc`/`kfree` heap allocator with corruption detection
- Serial (COM1) kernel logging
- A minimal interactive shell (`help`, `echo`, `meminfo`, `uptime`, `reboot`)

**Phase 3 - Paging, Physical Memory & Filesystem**
- Physical memory manager (bitmap frame allocator, real Multiboot memory map)
- Paging enabled (identity-mapped 0-64MB), with a page-fault handler
  that decodes the faulting address instead of triple-faulting
- ATA PIO disk driver + a read-only FAT32 filesystem
- Shell `ls`/`cat` commands, backed by a real disk image (`make disk.img`)

**Phase 4 - Usermode Processes, Syscalls & Scheduling**
- A Task State Segment and real ring-3 (CPL=3) process execution
- Preemptive round-robin scheduler (context switching via a hand-written
  stack-swap primitive)
- `int 0x80` syscall interface (`SYS_WRITE`/`SYS_EXIT`/`SYS_YIELD`) -
  the only path from ring 3 into the kernel
- A demo process that actually runs at ring 3 and talks to the kernel
  only through syscalls
- Shell `ps` command

**Phase 5 - Security Hardening: Address-Space Isolation**
- Every user process gets its own page directory (not a shared one)
- Private, PMM-backed process memory - proven, not just claimed: two
  demo processes use the *same* virtual stack address and verify at
  runtime that neither can see or corrupt the other's data
- CR3 switched on every context switch

**Phase 6 - Networking**
- NE2000 ISA NIC driver (polling PIO)
- Ethernet, ARP, IPv4, and ICMP (ping) - a real, verified round trip
  against QEMU's own network gateway, no host network access required
- Shell `ping IP` command; an automatic gateway ping at boot proves
  the whole stack headlessly, the same pattern every phase's self-test
  uses

**Phase 7 - Graphics Mode & a Minimal Windowing System**
- VGA Mode 13h (320x200x256) via direct register programming -
  switchable at runtime without disturbing the existing text-mode shell
- PS/2 mouse driver (IRQ12)
- A small compositor: 3 draggable, titled, colored windows
- Shell `gui` command (ESC to return to the text shell)

**Phase 8 - Package Manager (nova-pkg CLI)**
- FAT32 gained write support (create + delete files) - every phase
  before this was read-only
- `nova-pkg`: install/remove/list packages from a simple single-file
  `.PKG` format
- Shell `pkg list` / `pkg installed` / `pkg install NAME` /
  `pkg remove NAME`
- No GUI "Software Center" yet - see PROGRESS.md for why

**Phase 9 - First-Run Setup, RTC & Persistent Identity**
- A first-run wizard - the realistic "installer" for a live-boot OS -
  asks for a hostname/username once and persists them to disk
- CMOS real-time clock driver
- Personalized shell prompt (`user@host>`); `date`/`hostname`/`whoami`
- Persistence verified across an actual reboot, not just within one
  session - see PROGRESS.md

**Phase 10 - UDP, TFTP Client & Networked Package Fetching**
- Minimal UDP (send + single-listener receive), wired into the IPv4
  stack alongside ICMP
- A read-only TFTP client (RFC 1350)
- `pkg fetch NAME` downloads a package over the network and makes it
  installable - closing Phase 8's "nothing to fetch a package from
  yet" gap
- Not part of the original 9-phase plan - the first phase chosen from
  open follow-up items once that plan was complete (see PROGRESS.md)

**Phase 11 - Capability-Based File Access Control**
- Three new syscalls (`SYS_OPEN`/`SYS_READ`/`SYS_CLOSE`) - the first
  ring-3 code could ever touch the filesystem at all
- Every process is filesystem-capability-empty by default; only
  explicitly granted filenames can be opened, enforced in the kernel
- Proven with a falsifiable test, not just claimed: a demo process
  successfully reads an allowed file and is genuinely denied a
  disallowed one, logged independently by both the kernel's security
  check and the process's own self-verification
- Closes the "least-privilege process model" item from the security
  roadmap, deferred since Phase 5

**Phase 12 - GUI Software Center**
- 26 hand-built uppercase letter glyphs extend Phase 7's digit-only
  font, verified pixel-exact against the rendered output rather than
  by eye
- `store`: a GUI front-end for `nova-pkg` - lists packages with
  INSTALL/REMOVE buttons, wired to the real package manager functions
- Connects the package manager and windowing system for the first
  time, closing a gap from the project's original vision
- Click-driven install/remove not conclusively verified in headless
  testing (the same kind of gap Phase 7 flagged for window-dragging) -
  see PROGRESS.md

**Phase 13 - PCI Bus Enumeration**
- Real PCI configuration space access (32-bit port I/O) and full
  bus/device/function enumeration
- `lspci` shell command; a boot self-test that needs no disk or NIC
  attached to produce a verifiable result
- Correctly identifies real hardware: Intel's 82441FX host bridge,
  PIIX3 ISA/IDE bridges, and QEMU's own virtual VGA adapter
- Foundational for future driver work (USB, sound, additional NICs)
  rather than a driver itself - detection only, no new PCI-based
  hardware support

**Phases 14-16 - Network Capabilities, Font Polish & a Second NIC (delivered together)**
- Phase 14: `SYS_NET_SEND` extends Phase 11's capability model to
  network destinations - a process can only send UDP packets to IPs
  it was explicitly granted, enforced in the kernel
- Phase 15: added missing punctuation (parentheses, `!`, `,`) and the
  Software Center now shows full package descriptions, not just names
- Phase 16: a full RTL8139 PCI NIC driver, found via Phase 13's
  enumeration rather than a fixed address - the entire existing
  network stack (ARP, ICMP ping, TFTP) now runs correctly over it,
  proving PCI detection leads to real usable hardware support

**Phases 17-19 - Spawn Capability, Sound & DNS (delivered together)**
- Phase 17: `SYS_SPAWN` extends the capability model a third time to
  process creation - the spawned process genuinely runs independently
  (own PID, own output), not just a returned success code
- Phase 18: a full AC97 PCI sound driver, found by PCI class code
  rather than a specific vendor ID - verified by capturing and
  analyzing real PCM audio output, which also caught and fixed a real
  bug (replaying a beep after the first one finished produced silence)
- Phase 19: a minimal DNS client - `ping`/`nslookup` now resolve real
  hostnames, verified against actual external DNS resolution, not just
  protocol-level correctness

**Phases 20-22 - Release: Installable Image, Licensing & Hardening**
- Phase 20: `novaos.iso` is a hybrid image - verified to boot as a
  real BIOS disk/USB image with no CD-ROM emulation at all, using
  GRUB's own proven tooling rather than a custom bootloader. `make
  install-image` prints the exact steps to install it.
- Phase 21: MIT license, version bumped to 1.0.0, and a new
  [CHANGELOG.md](CHANGELOG.md) summarizing the full release.
- Phase 22: fixed a real, long-documented resource leak - every
  process's kernel stack, user stack, and page directory now get freed
  on exit, validated against the project's most demanding existing
  test coverage (five process exits every single boot).

**Phase 23 - ELF Loading & a Real Process Model**
- NovaOS can now load and run a real, independently-compiled ELF32
  executable from disk - previously, every process ran C functions
  compiled directly into the kernel image
- Real `argv`/exit-code semantics using the actual x86 process-entry
  stack convention, not a simplified placeholder - verified with a
  genuine standalone test executable that echoes its own arguments and
  exits with a specific, checkable code
- `run PATH [args...]` in the shell; `SYS_EXEC`/`SYS_WAIT` syscalls,
  reusing Phase 17's spawn capability
- Deliberately `exec`-style (spawn-and-load), not true `fork()` - see
  PROGRESS.md for why, stated upfront rather than glossed over
- Found and fixed a real correctness bug before it shipped: process
  exit cleanup didn't know about ELF segment memory, generalized to
  correctly free any address space layout

**Phase 24 - Minimal Libc Port**
- A real (if intentionally small) C standard library:
  `printf`/`puts`/`putchar`, string functions, and a `malloc`/`free`
  backed by a new `SYS_SBRK` syscall - user processes previously had
  no heap at all
- `crt0.asm` bridges NovaOS's process-entry stack convention into a
  proper `int main(int argc, char** argv, char** envp)` call
- Verified end-to-end with a real C test program chaining `malloc` ->
  `strcpy` -> `strcat` -> `printf("%s")` together - the exact string
  produced would not come out intact if any piece had a bug
- Worked correctly on the first boot test - genuinely the highest-risk
  phase for a subtle mistake so far (crt0/`printf`/`malloc` are
  classic sources of hard-to-spot bugs), and it didn't have one

**Phase 25 - MBR/GPT Partitions & a Real Filesystem (ext2)**
- `disk.img` is now a genuinely partitioned disk (MBR, verified with
  real `parted`-built partition tables), not one bare unpartitioned
  filesystem - closing a real, previously-documented gap
- A real, from-scratch, read-only ext2 driver, verified against
  filesystem images built by actual Linux tools (`mkfs.ext2`,
  `debugfs`), not just this driver's own round-trip
- `cat`/`run`/every existing command transparently works with ext2
  files too - `vfs_read_file()` falls back to ext2 if FAT32 doesn't
  have the file, no new shell commands needed
- FAT32 was retrofitted for partition-awareness with **zero changes
  to its internals** - a single partition offset added in `ata.c`
  that FAT32's five public entry points set themselves, keeping the
  retrofit low-risk to code every other phase depends on
- Zero regression, verified against the existing FAT32 self-test
  passing byte-exact despite every disk access now going through a
  partition offset it didn't have before

**Phases 26-27 - ext2 Write Support & True `fork()` (delivered together)**
- Phase 26: ext2 can now write files, not just read them - block/inode
  allocation and directory-entry insertion, verified by inspecting the
  result with real, independent `debugfs` tooling (not this project's
  own code) after a boot self-test wrote a new file
- Phase 27: genuine `fork()` semantics via copy-on-write - not the
  `exec`-style spawn Phase 23 deliberately used instead. A forked
  child resumes execution at the *exact point* the parent called
  `fork()` from, sharing memory with the parent until either side
  writes to it
- The fork() test proves actual process isolation, not just that it
  runs: a child modifies its own copy of a stack variable and exits
  with a distinct code; the parent then confirms its *own* copy is
  untouched - real proof copy-on-write correctly separated the two
  processes' memory
- Worked correctly on the first boot test - remarkable given this is
  among the most structurally delicate mechanisms possible in a
  kernel (hand-constructed interrupt return frames, copy-on-write
  with correct TLB management), stated plainly rather than downplayed

**Phase 28a - Minimal TCP Client**
- A real TCP client (3-way handshake, real checksums, a 4-way close) -
  deferred three times previously (Phases 6, 10, 19) for retransmission/
  state-machine complexity, closed now with a deliberately scoped-down
  but genuinely functional implementation
- Found and fixed a real, pre-existing bug: `ip_send()` never actually
  used the gateway for off-subnet destinations - `NET_NETMASK` had
  been defined since Phase 6 and never once used, since every earlier
  self-test only ever talked to on-subnet addresses
- Verified at the **packet level** against a real, external,
  unmodified server: captured traffic shows a correct handshake, our
  56-byte HTTP request, the server's 259-byte response reassembled
  with zero corruption, and a clean close - not just a self-reported
  success log line
- One connection at a time, stop-and-wait (no retransmission or
  sliding window) - honestly scoped, not hidden

**Phase 28b - UHCI USB Controller & Device Enumeration**
- A real UHCI (USB 1.1) host controller driver - the simplest of the
  four USB controller interfaces, with an I/O-port register model
  consistent with every other driver in this tree
- Full device enumeration via real control transfers (SETUP/DATA/
  STATUS staging, `GET_DESCRIPTOR`, `SET_ADDRESS`) - verified against
  **real QEMU USB hardware emulation**: `vendor=0x627` is QEMU's own
  USB vendor ID, independent confirmation this is a genuinely decoded
  device descriptor, not a fabricated success
- Found and fixed a classic systems-programming bug through
  diagnostic-driven debugging: DMA-shared transfer descriptors
  weren't marked `volatile`, so at this project's `-O2` optimization
  level the compiler treated the poll loop's condition as constant
  instead of re-reading hardware-modified memory
- USB is now a standard, always-attached, hard-asserted part of every
  boot including `make test` - unlike DNS/TCP, this has no external-
  connectivity dependency
- HID keyboard report reading is explicitly out of scope for this
  pass (documented as a clear follow-up) - enumeration is the
  verified, complete milestone here

**Phase 28c - A Real, From-Scratch Bootloader**
- A genuine two-stage real-mode bootloader (`tools/custom-boot/`) -
  the first real-mode/BIOS-interrupt code ever written in this
  project. BIOS E820 memory detection, LBA disk reads, A20 enable, a
  flat GDT, the protected-mode transition, and 32-bit ELF program-
  header parsing to load the *exact same* kernel binary GRUB boots,
  with zero kernel-side changes needed
- **Purely additive**: a completely separate, parallel boot path -
  `novaos.iso` and `disk.img`'s normal build are untouched. `make
  test-custom-boot` runs it independently
- The critical proof point: `PMM initialized (131040 frames tracked,
  511MB)` exactly matches the `-m 512M` QEMU flag, confirming the
  E820-to-Multiboot memory map this bootloader builds is genuinely
  correct - without it, every feature built since Phase 5 (paging,
  `fork()`, `malloc`) would silently break even if the kernel
  appeared to boot
- Verified with the complete self-test suite (network, USB, the
  FAT32+ext2 data disk): 67 `PASS`/`[ OK ]` lines, zero failures,
  including ext2 read/write, USB enumeration, real TCP/HTTP, and
  `fork()`+copy-on-write - every feature from every prior phase works
  identically to a GRUB boot
- Two real bugs (an off-by-one LBA calculation, a structural padding-
  order issue) were caught through careful review *before* ever
  booting real-mode code - the appropriate order of operations for
  code this unforgiving of mistakes

**Phase 29 - Kernel/Userland Architectural Separation**
- Addressed a real, correctly-identified architectural gap:
  `kernel/shell/`, `kernel/gui/`, `kernel/pkg/` were compiled directly
  into the kernel binary, running in ring 0 - nothing like the clean
  Linux-kernel-vs-Ubuntu-userland split those names implied
- All three physically moved to `userland/`, with every include path
  individually reasoned about (not a blind find-and-replace) - full
  clean rebuild, zero regressions, both `make test` and `make
  test-custom-boot` still pass
- **Honestly, this alone is organizational, not a new security
  property** - `shell`/`gui`/`pkg` still run in ring 0. The real proof
  is `userland/coreutils/cat.c`: a genuinely separate, standalone
  ELF32 executable talking to the kernel *only* through syscalls -
  the actual category of thing `cat` is to the Linux kernel
- Surfaced and resolved a real design tension: `process_exec()`
  deliberately grants no file capabilities by default (Phase 11's
  least-privilege model), so a new `process_exec_with_files()` lets a
  trusted, ring-0 caller grant specific access at exec time, without
  weakening what ring-3 `SYS_EXEC` itself can grant
- Found and fixed a real bug through the same debugging discipline
  used throughout this project: the first version hung the boot by
  calling a blocking wait before the scheduler had started - the
  exact mistake previously identified and avoided in Phase 23,
  caught immediately and fixed the same way

**Phase 30 - Genuine Ring-3 Shell (Kernel Independence, Completed)**
- The real completion of Phase 29's architectural point: NovaOS now
  **boots into a genuine ring-3 shell** -
  `userland/ring3-shell/shell.c`, a completely separate ELF32
  executable talking to the kernel only through syscalls - not a
  ring-0 kernel task. The same relationship `bash` has with the Linux
  kernel on Ubuntu
- Two new syscalls, each built with real care about a correctness
  hazard: `SYS_READ_KEY` is deliberately non-blocking, since a
  syscall runs with interrupts disabled and the existing keyboard
  driver's blocking read would deadlock the kernel if called
  unconditionally; `SYS_LIST_FILES` powers `ls`
- A new, narrow capability grant (`can_open_any_file` +
  `can_spawn`, via `process_exec_as_shell()`) lets the shell open
  whatever file the user names and run other programs - reserved for
  the shell specifically, never reachable from ring-3 `SYS_EXEC`
- **Verified through genuine interactive use**, not just automated
  self-tests - real typed commands via QEMU monitor keystrokes,
  screendumped and log-verified: `ls`, `cat`, `echo`, and `run`
  (loading and waiting on a real ELF program) all confirmed working
  end-to-end
- Two real bugs found and fixed (a self-inflicted duplicate
  assignment from an earlier automated edit; a missing `can_spawn`
  grant) plus one correctly-diagnosed test-tooling discovery (QEMU's
  `sendkey` routing to a USB keyboard instead of PS/2 when both are
  attached) - each found through the same systematic debugging
  discipline used throughout this project
- **Honestly scoped**: `ping`/`pkg`/the GUI aren't in the ring-3 shell
  yet - each needs its own new syscall surface not built in this pass
  (`date`/`lspci`/`beep` were added next, in Phase 31). See
  PROGRESS.md for the full, honest limitations list

**Interim fix - USB busy-wait timing**
- A real `make test` failure reported on a real machine (not this
  project's usual sandboxed testing environment): the boot log
  stopped right after `AC97 beep: playing...`, with none of the later
  self-tests ever appearing before the timeout
- Root cause: `usb_uhci_init()`'s port-reset delays used raw
  instruction-count busy-wait loops instead of this kernel's own
  timer-based delay - wall-clock duration for a raw instruction count
  varies unpredictably with host CPU speed, so the exact same code
  that finishes fine on one machine can push boot time past
  `TEST_TIMEOUT` on a slower one
- Fixed by using `timer_sleep_ms()` (the same mechanism every other
  timeout in this kernel already relies on) instead; also increased
  `TEST_TIMEOUT` from 15s to 25s as a defensive margin given how much
  the self-test suite has grown across 30 phases

**Phase 31 - Restoring Shell Command Parity (date/lspci/beep)**
- The first concrete step restoring the ring-3 shell's command set:
  three new syscalls (`SYS_RTC_READ`, `SYS_LSPCI`, `SYS_BEEP`) for
  the simplest, lowest-risk commands - reading existing kernel state,
  no new capability model needed (unlike `ping`'s request/reply
  timeout semantics or `pkg`'s install/remove state, both deliberately
  left as harder, separate follow-up work)
- Verified through real interactive testing: `date`, `lspci`, and
  `beep` all confirmed working, including a screendump specifically
  taken to resolve an apparent discrepancy in the debug log (traced
  to a cosmetic 256-byte limit in the kernel's own debug-logging
  helper, unrelated to the actual `SYS_WRITE`/`SYS_LSPCI`
  functionality, which was confirmed complete and correct on the real
  screen)

**Phase 32a - Package Manager Converted to Ring-3**
- `pkg list`/`install`/`remove` reimplemented from scratch in
  `userland/ring3-shell/shell.c`, using only two new syscalls
  (`SYS_WRITE_FILE`, `SYS_DELETE_FILE`) - zero calls into any
  kernel-side pkg-specific code
- A shell builtin, not a separate exec'd `pkg.elf`, by deliberate
  choice: exec'd programs start with zero capabilities, and there's
  no mechanism yet for the shell to delegate a subset of its own
  broad access to something it execs - building that properly is a
  real, separate design question, not rushed into this pass
- Verified through the complete real-world lifecycle: install, list
  (showing `[installed]`), remove, list again - file sizes matched
  exactly (62 bytes = the package's own payload size, 101 bytes = one
  install record), screendumped for a definitive check

**Phase 32b - Foundational Ring-3 Graphics + a Real VGA Bug Fix**
- Five new syscalls (`SYS_GFX_ENTER/EXIT`, `SYS_GFX_PUT_PIXEL`,
  `SYS_GFX_FILL_RECT`, `SYS_MOUSE_READ`) and a genuine, verified-
  working ring-3 graphics demo (`gui.c`) - deliberately a
  proof-of-concept scene, **not** a port of the existing multi-window
  compositor or the Store's package-browsing UI, which remain ring-0
- **A real bug found through more rigorous testing than this project
  had done before**: graphics rendered perfectly, but the screen
  turned into unreadable stripes immediately after returning to text
  mode - while the system stayed 100% functional underneath the
  whole time (confirmed via serial log). Root cause: VGA Mode 13h's
  Chain-4 addressing touches all 4 memory planes on every framebuffer
  write, silently destroying the font bitmap data text mode needs
  afterward
- Investigated, not guessed: a first hypothesis (a missing VGA
  synchronous-reset step) was implemented and screendump-tested, and
  found *not* to be the actual cause - kept anyway as a legitimate,
  separate best practice - before the real cause (font-plane
  corruption) was confirmed and fixed, including catching a second
  bug in the fix itself (leftover registers not restored) the same
  way: by checking the actual screendump, not assuming success once
  it compiled
- Verified with a completely clean, readable screen immediately after
  `gui` exits and after a subsequent `ls`

See [PROGRESS.md](PROGRESS.md) for verification details and known
limitations of the current build.

## Quick Start

Full instructions for Windows (WSL2), Linux, macOS (Intel & Apple
Silicon), and Docker are in **[TESTING.md](TESTING.md)**. Short version
for Linux/macOS:

```bash
make setup   # install dependencies for your OS
make         # build novaos.iso
make run     # boot it in QEMU
make test    # headless boot smoke test (what CI runs)
```

On Windows, install WSL2 (`wsl --install`) and run the same commands
inside the Ubuntu shell it gives you - see TESTING.md for details.

## Project Layout

```
novaos/
├── kernel/             # Phase 29: the TRUE kernel only - boot/arch/drivers/fs/net/task/mm, nothing userland-conceptual
│   ├── arch/x86/
│   │   ├── boot/       # Multiboot entry point
│   │   ├── cpu/        # GDT, TSS, IDT, ISR, IRQ, syscall, context switch
│   │   └── mm/         # PMM, paging, heap allocator
│   ├── drivers/        # vga, serial, timer, keyboard, ata, net (ne2000, rtl8139), video (Mode 13h), mouse (PS/2), rtc, pci, sound (ac97), usb (uhci)
│   ├── fs/             # VFS (FAT32 + ext2 fallback), FAT32 (read + write), ext2 (read-only), MBR/GPT partitions
│   ├── net/            # ethernet, arp, ipv4, icmp, udp, tftp, dns, tcp
│   ├── config/         # persistent system identity (hostname/username)
│   ├── task/           # process table, scheduler, ELF loader, syscall + sandbox/greeter/unprivileged demo tasks
│   ├── lib/            # freestanding string/stdio subset (kernel-internal, separate from userland/libc/)
│   ├── include/        # public kernel headers
│   └── init/           # kernel_main and init sequencing
├── userland/           # Phase 24 libc + Phase 29 kernel/userland separation + Phase 30 ring-3 shell
│   ├── libc/           # crt0, syscalls, string/stdio/stdlib - what a real ring-3 program links against
│   ├── ring3-shell/    # Phase 30: the genuine ring-3 shell NovaOS boots into - real ELF, syscalls only
│   ├── coreutils/      # Phase 29: genuine ring-3 ELF programs (cat) - talk to the kernel only via syscalls; Phase 32b adds gui.c, a graphics/mouse syscall demo
│   ├── examples/       # Phase 24 example C programs
│   ├── gui/            # compositor (windowing demo), store (Software Center), font + canvas - still ring-0 for now, see PROGRESS.md
│   ├── pkg/            # nova-pkg package manager - still ring-0 for now
│   └── shell/          # Phase 3-era ring-0 shell - kept for reference, no longer launched at boot (see Phase 30), still ring-0
├── tools/              # linker script, grub.cfg, FAT32 test fixtures, ELF test fixture sources, custom-boot/ (Phase 28c bootloader)
├── scripts/            # per-OS setup scripts
├── .github/workflows/  # CI (build + make test on every push)
├── Dockerfile          # reproducible cross-platform build environment
├── PROJECT_PLAN.md     # vision, architecture, phased roadmap
├── PROGRESS.md         # what's actually built vs. planned
└── TESTING.md          # cross-platform build/test/debug guide
```

## Documentation

- [PROJECT_PLAN.md](PROJECT_PLAN.md) - vision, security roadmap, phases
- [PROGRESS.md](PROGRESS.md) - implementation status, verified behavior, known limitations
- [TESTING.md](TESTING.md) - build/test/debug on Windows, Linux, macOS, and Docker

## License

Add a LICENSE file appropriate to your goals for the project (MIT/GPL/etc. -
none is currently specified in this repository).
