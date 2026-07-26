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
|  | Security Layer: capabilities, sandboxing, MAC       |     |  Phase 5
|  +----------------------------------------------------+     |
|  | Process Scheduler + Syscall Interface                |     |  Phase 4
|  +----------------------------------------------------+     |
|  | Filesystem (VFS + FAT32, later ext-like)             |     |  Phase 3
|  +----------------------------------------------------+     |
|  | Device Drivers: VGA/Serial/PS2/PIT/ATA/PCI/NIC       |     |  Phase 1-3,6
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
| Paging + per-process address spaces (no process can read another's memory) | 3 | Planned |
| NX (non-executable) data pages - stack/heap can't be executed as code | 3 | Planned |
| Syscall-gated kernel entry (user code can't call kernel functions directly) | 4 | Planned |
| Least-privilege process model (capabilities, not raw root/non-root) | 5 | Planned |
| Mandatory sandboxing for GUI apps (Android/iOS-style permission prompts, not Windows-style "click Yes and hope") | 5 | Planned |
| Signed packages + signature verification in the package manager | 8 | Planned |
| Verified/measured boot (GRUB to kernel signature check) | 9 | Stretch goal |
| Disk encryption offered at install time | 9 | Stretch goal |

## 5. Usability Layer (roadmap)

| Feature | Phase | Notes |
|---|---|---|
| Interactive kernel shell | 2 | Done (`help`, `echo`, `meminfo`, `uptime`, `reboot`) |
| Full POSIX-ish shell (`ls`, `cat`, pipes, scripts) | 4 | Needs the Phase 3 filesystem first |
| `nova-pkg`: apt-style package manager (CLI) | 8 | Dependency resolution, signed packages |
| "Software Center": GUI front-end for `nova-pkg` | 8 | The literal "Ubuntu-like" ask |
| Windowing system / desktop shell | 7 | Framebuffer graphics mode, compositor, taskbar |
| First-run setup wizard (locale, user account, Wi-Fi) | 9 | The "Windows OOBE"-like ask |
| Automatic security update service | 8-9 | Opt-out, not opt-in, by default |

## 6. Development Phases

| Phase | Focus | Status |
|---|---|---|
| **P1** | Bootloader & Kernel Foundation | Complete |
| **P2** | Memory Management & Interrupts | Complete (this update) |
| **P3** | Paging, heap hardening, filesystem (VFS + FAT32), storage (ATA) drivers | Next |
| **P4** | Usermode processes, syscalls, scheduler, real shell | Planned |
| **P5** | Security hardening layer (capabilities, sandboxing) | Planned |
| **P6** | Networking (NE2000/virtio-net, TCP/IP, sockets) | Planned |
| **P7** | Graphics mode + windowing system | Planned |
| **P8** | Package manager + Software Center ("the distro layer") | Planned |
| **P9** | Installer, first-run wizard, driver support, public release polish | Planned |

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
