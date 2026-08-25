# NovaOS - Project Plan

**Version:** 2.0
**Date:** July 2026
**Author:** Firoze (Creator of the Roze language)
**Supersedes:** the original v1.0 plan (kept for history; see `docs/legacy/`)

---

## 1. Vision

NovaOS is a from-scratch, x86 operating system built in C and Assembly.
The goal is not to clone any single existing OS, but to combine the
parts of three different traditions that make the most sense for a
modern hobby-to-production OS:

| Borrowed from | What, specifically | Why |
|---|---|---|
| **Linux** | A modular monolithic kernel: drivers, filesystem, and network stack as loosely-coupled subsystems behind stable internal interfaces (VFS, driver registration, syscalls) | Proven architecture, easy to extend one subsystem without rewriting others |
| **Ubuntu (Debian-family distros)** | A friendly userland *on top of* the kernel: a package manager, a "Software Center"-style GUI installer, and sane defaults so a non-technical user doesn't need a terminal to get work done | Kernel quality alone doesn't make an OS usable - the distro layer is what people actually experience |
| **Windows** | UX conventions people already know (a taskbar/start-menu metaphor, a permission-prompt model for privileged actions, an automatic-update service) and a driver model that treats "install a device" as a solved, boring problem | Lowest switching cost for the majority of users who've only used Windows |

**What this does *not* mean:** NovaOS will not run Linux `.deb`/`.rpm`
binaries or Windows `.exe` files unmodified. Binary compatibility with
either ecosystem (a la Wine or WSL) is an enormous, multi-year
undertaking on its own and is explicitly **out of scope**. "Combine
Linux and Windows" here means *borrowing proven architecture and UX
ideas*, not binary compatibility. Being upfront about that scope now
avoids setting an expectation the project can't deliver on.

## 2. Design Pillars

1. **Secure by default.** Every privilege boundary (ring 0/3, process
   isolation, filesystem permissions, package signing) is designed in
   from the start rather than retrofitted. See section 4.
2. **Usable by default.** A fresh install should be usable by someone
   who has never opened a terminal: GUI installer, GUI package manager,
   sensible defaults, clear error messages instead of raw fault dumps.
3. **Transparent and inspectable.** Every subsystem logs what it's
   doing (see the serial `kernel_log` convention introduced in Phase 2)
   so both developers and, eventually, an end-user "System Log" app can
   see what the OS is doing and why.
4. **Incrementally real.** Every phase ends with something that
   actually boots and is boot-tested in CI (`make test`), not a stack
   of header files with no working implementation behind them.

## 3. Technical Architecture

```
+-------------------------------------------------------------+
| USER SPACE (Phase 4+)                                       |
|  +-----------+  +------------+  +-----------+  +---------+  |
|  |  Shell /  |  |  Software  |  |    GUI    |  |  Roze   |  |
|  |  CLI apps |  |  Center    |  |  Desktop  |  | Runtime |  |
|  +-----------+  +------------+  +-----------+  +---------+  |
+-------------------------------------------------------------+
| KERNEL SPACE                                                 |
|  +----------------------------------------------------+     |
|  | Security Layer: address-space isolation [done], capabilities/sandboxing [planned] |     |  Phase 5+
|  +----------------------------------------------------+     |
|  | Process Scheduler + Syscall Interface                |     |  Phase 4 [done]
|  +----------------------------------------------------+     |
|  | Filesystem (VFS + FAT32, later ext-like)             |     |  Phase 3
|  +----------------------------------------------------+     |
|  | Device Drivers: VGA/Serial/PS2/PIT/ATA/NIC(ISA+PCI)/AC97 [done]; PCI enumeration [done] |     |  Phase 1-3,6,13,16,18
|  +----------------------------------------------------+     |
|  | Memory Management: GDT, paging, heap                 |     |  Phase 2-3
|  +----------------------------------------------------+     |
|  | Interrupts: IDT, ISR (exceptions), IRQ (PIC/APIC)    |     |  Phase 2 [done]
|  +----------------------------------------------------+     |
|  | Bootloader (GRUB / Multiboot)                        |     |  Phase 1 [done]
|  +----------------------------------------------------+     |
+-------------------------------------------------------------+
```

## 4. Security Architecture (roadmap)

Security is treated as a first-class subsystem, not a checklist added
at the end. Concretely, in the order it gets built:

| Mechanism | Phase | Status |
|---|---|---|
| Ring 0 / ring 3 GDT segments | 2 | Done - segments exist; nothing runs in ring 3 yet |
| Masked-by-default IRQs (a driver must opt in) | 2 | Done |
| Fault isolation: CPU exceptions panic cleanly with a full register dump instead of silently corrupting state | 2 | Done |
| Paging (single flat 64MB address space) | 3 | Done |
| Ring 3 execution + preemptive scheduling | 4 | Done - real CPL=3 processes, round-robin scheduler; see PROGRESS.md |
| Syscall-gated kernel entry (user code can't call kernel functions directly) | 4 | Done - int 0x80, DPL=3 gate is the only path from ring 3 into the kernel |
| Per-process address spaces (no process can read another's memory) | 5 | Done - each process gets its own page directory and private stack; proven by a boot-time test, not just claimed (see PROGRESS.md) |
| Least-privilege process model (capabilities, not raw root/non-root) | 11, 14, 17 | Done (scoped) - syscall-gated, per-process capability lists for files (P11), network destinations (P14), and process creation (P17); proven with falsifiable allow/deny tests, not just claimed (see PROGRESS.md). Not extended to other resource types yet |
| Mandatory sandboxing for GUI apps | 7+ | Planned - needs a GUI to sandbox in the first place |
| NX (non-executable) data pages - stack/heap can't be executed as code | - | Deferred - needs PAE or long mode, which 32-bit non-PAE paging (built in Phase 3) doesn't have |
| Signed packages + signature verification in the package manager | 9+ | Planned - Phase 8 built the package manager itself with no signing yet |
| Verified/measured boot (GRUB to kernel signature check) | 9 | Stretch goal |
| Disk encryption offered at install time | 9 | Stretch goal |

## 5. Usability Layer (roadmap)

| Feature | Phase | Notes |
|---|---|---|
| Interactive kernel shell | 2 | Done (`help`, `echo`, `meminfo`, `uptime`, `reboot`) |
| Filesystem-backed shell commands (`ls`, `cat`) | 3 | Done |
| Process listing (`ps`) | 4 | Done |
| Full POSIX-ish shell (pipes, scripts, job control) | - | Not yet planned in detail; needs a real filesystem-backed multi-process model first (Phase 4 process/syscall groundwork exists; a spawn/exec syscall doesn't yet) |
| `nova-pkg`: CLI package manager | 8 | Done (scoped) - install/remove/list a single-file `.PKG` format; no dependency resolution, no signing; network fetch added in Phase 10 (TFTP only, see PROGRESS.md) |
| "Software Center": GUI front-end for `nova-pkg` | 12 | Done (scoped) - lists packages, click INSTALL/REMOVE calls the real pkg_install()/pkg_remove(); a small hand-built font (26 letters + digits) rather than general text rendering. Click-driven interaction not conclusively verified in headless testing - see PROGRESS.md |
| Windowing system / desktop shell | 7 | Done (scoped) - VGA Mode 13h graphics, a 3-window compositor, PS/2 mouse; no taskbar, no general text rendering, no window create/close yet |
| First-run setup wizard (hostname, username) | 9 | Done (scoped) - no locale, Wi-Fi (no wireless driver exists), or disk partitioning step; see PROGRESS.md |
| Automatic security update service | 8-9 | Opt-out, not opt-in, by default |

## 6. Development Phases

| Phase | Focus | Status |
|---|---|---|
| **P1** | Bootloader & Kernel Foundation | Complete |
| **P2** | Memory Management & Interrupts | Complete (this update) |
| **P3** | Paging, physical memory management, filesystem (VFS + FAT32, read-only), ATA storage driver | Complete (scoped - see PROGRESS.md) |
| **P4** | Usermode processes, syscalls, scheduler, real shell | Complete (scoped - see PROGRESS.md) |
| **P5** | Security hardening: per-process address space isolation | Complete (scoped - see PROGRESS.md) |
| **P6** | Networking: NE2000 driver, Ethernet/ARP/IPv4/ICMP | Complete (scoped - see PROGRESS.md; UDP added in P10, TCP/sockets still deferred) |
| **P7** | Graphics mode (VGA Mode 13h) + minimal windowing, PS/2 mouse | Complete (scoped - see PROGRESS.md; drag-and-drop needs manual confirmation) |
| **P8** | Package manager: `nova-pkg` CLI + FAT32 write support | Complete (scoped - see PROGRESS.md; network fetch added in P10, GUI Software Center added in P12) |
| **P9** | First-run setup wizard (the realistic "installer" for a live-boot design), RTC driver, persistent identity | Complete (scoped - see PROGRESS.md; no bootloader-writing installer) |
| **P10** | UDP + a TFTP client + networked package fetching for `nova-pkg` | Complete (scoped - see PROGRESS.md) |
| **P11** | Capability-based file access control for ring-3 processes | Complete (scoped - see PROGRESS.md) |
| **P12** | GUI Software Center - connects `nova-pkg` to the windowing system | Complete (scoped - see PROGRESS.md) |
| **P13** | PCI bus enumeration | Complete (scoped - see PROGRESS.md; detection only - a PCI-based driver followed in P16) |
| **P14** | Extend capability-based access control to network destinations | Complete (scoped - see PROGRESS.md) |
| **P15** | Font punctuation + package descriptions in the Software Center | Complete (scoped - see PROGRESS.md) |
| **P16** | RTL8139 PCI NIC driver | Complete (scoped - see PROGRESS.md) |
| **P17** | Extend capability-based access control to process creation | Complete (scoped - see PROGRESS.md) |
| **P18** | AC97 PCI sound driver | Complete (scoped - see PROGRESS.md) |
| **P19** | Minimal DNS client | Complete (scoped - see PROGRESS.md) |
| **P20** | Installable disk image (GRUB hybrid ISO, not a custom bootloader) | Complete (scoped - see PROGRESS.md) |
| **P21** | License, versioning (1.0.0), and a changelog | Complete - see PROGRESS.md and CHANGELOG.md |
| **P22** | Process-exit resource cleanup (real leak fix) | Complete - see PROGRESS.md |
| **P23** | ELF loading + a real process model (argv/exit codes/SYS_EXEC/SYS_WAIT) | Complete (scoped - see PROGRESS.md; exec-style, not true fork()) |
| **P24** | Minimal libc port (crt0, printf, malloc via new SYS_SBRK, string functions) | Complete (scoped - see PROGRESS.md) |
| **P25** | MBR/GPT partition support + a real second filesystem (ext2, read-only) | Complete (scoped - see PROGRESS.md) |

Phase 25 (post-release) is the third and final item from the
gap-analysis roadmap - disk.img is now genuinely partitioned rather
than one bare filesystem, and NovaOS can read a real ext2 filesystem
alongside FAT32, verified against images built by actual Linux tools.

Phase 24 (post-release) is the second item from the gap-analysis
roadmap - makes ELF-loaded programs (P23) actually useful by giving
them a real C standard library instead of requiring hand-written
assembly for every program.

**NovaOS 1.0.0 was released after Phase 22.** See CHANGELOG.md for the
full release summary. Phase 23 (post-release) is the first item from
the separate NovaOS-vs-Ubuntu gap-analysis roadmap: real ELF loading,
the single highest-leverage capability identified there.

**P1-P9 above were the full originally-planned roadmap; all nine
completed it at the scoped level described in each row and in
PROGRESS.md.** P10 through P19 were each chosen from open follow-up
items rather than a pre-written plan, since none exists past P9: P10
closed P6's deferred UDP and P8's deferred network-fetch gap together;
P11 closed the capabilities/least-privilege item from the security
roadmap in section 4; P12 closed the GUI Software Center gap by
building a small hand-crafted font rather than waiting for general
text rendering; P13 added PCI device detection, foundational for any
future USB/sound/additional-NIC driver work; P14, P15, and P16 were
delivered together in one batch - P14 extended P11's capability model
to network destinations, P15 rounded out the font with punctuation
real package descriptions actually use, and P16 proved P13's PCI
detection leads to actual usable hardware support by adding a full
second NIC driver; P17, P18, and P19 were delivered together in a
second batch - P17 extended the capability model a third time to
process creation, P18 added a full second PCI driver class (AC97
audio) proving the "detect then drive" pattern generalizes beyond
networking, and P19 added hostname resolution as a lower-risk
alternative to tackling full TCP/sockets. Further work (a real
bootloader-writing installer, TCP/sockets, extending capability-based
access control to further resources, true lowercase font forms, and
USB drivers) remains open and should each be scoped on their own terms
rather than assumed as "next."

Detailed, living status for what's actually implemented (as opposed to
planned) lives in **PROGRESS.md**, which is updated every phase instead
of only at planning time.

## 7. Testing Strategy

- **`make test`** - headless boot smoke test, asserts the serial boot
  log reaches `Interrupts enabled` with no `PANIC`/`FAULT`. Runs in
  GitHub Actions CI on every push/PR (`.github/workflows/ci.yml`).
- **`make debug`** - QEMU + GDB for investigating a specific failure.
- **Manual interactive checklist** - see TESTING.md - for anything
  that needs a keyboard/screen (shell commands, scrolling, etc.), since
  headless CI can't exercise those.
- Going forward, each phase's pull request is expected to extend
  `make test`'s assertions to cover that phase's new subsystem (e.g.
  Phase 3 should assert a file was successfully read from a FAT32 test
  image).

## 8. Contribution Workflow

1. Branch per phase/feature: `feature/phase3-filesystem`, etc.
2. Small, logically-scoped commits (see the Phase 2 commit history for
   the target granularity: one commit per subsystem, not one giant
   commit per phase).
3. `make test` must pass locally before opening a PR; CI re-verifies it.
4. Update `PROGRESS.md` in the same PR as the code it describes -
   documentation written after the fact tends not to happen at all.

## 9. Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Scope creep (this plan is ambitious) | High | High | Phases are ordered so each one is independently useful and shippable; a GUI-less, package-manager-less NovaOS is still a valid, demoable OS after Phase 4 |
| Cross-platform toolchain pain (especially Apple Silicon) | Medium | Medium | Docker-based build path (see TESTING.md) sidesteps host toolchain issues entirely |
| Security features bolted on late | Medium | Critical | Security roadmap (section 4) is interleaved with functional phases, not deferred to "Phase 10" |
| Setting unrealistic "runs Windows/Linux apps" expectations | Medium | High (reputational) | Explicitly scoped out in section 1; UX/architecture inspiration only |

## 10. Resources

Unchanged from the original plan - the OSDev Wiki, the Multiboot
specification, Intel SDMs, and standard OS textbooks (Tanenbaum,
Bovet & Cesati) remain the primary references.
