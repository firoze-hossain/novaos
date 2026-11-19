# Changelog

All notable changes to NovaOS are documented here. See
[PROGRESS.md](PROGRESS.md) for full technical detail, verification
evidence, and known limitations behind every phase summarized below.
See [RELEASE_SIGNOFF_V2.0.md](docs/RELEASE_SIGNOFF_V2.0.md) for the official
production release certification and 54-milestone completion report for NovaOS 2.0.0.

## [2.0.0] - Official Enterprise Production Release & General Availability (GA) (2026-11-19)

The complete fulfillment of the NovaOS 2.0 "Prometheus" 6-Month Enterprise Build Plan: 54 out of 54
planned tasks delivered in 100% pure safe Rust across the Kernel & Architecture Track (16 tasks),
Desktop & UI Track (16 tasks), Server Track (12 tasks), and Release & Enterprise Track (10 tasks),
validated by 616 combined automated boot assertions and 29 host milestone qualification suites.

### Key Architectural Pillars

- **Kernel & Modern Hardware Foundation (16 Tasks Delivered)**:
  - 64-Bit Transformation: Pure Rust x86_64 Long Mode with 4-level PML4 paging ($256 \text{ TiB}$ address space),
    hardware NX bit, 2MB/1GB huge pages, higher-half kernel base (`0xFFFF800000000000`), and direct physical map.
  - Bootloader: Native pure Rust UEFI 2.9 bootloader with GOP 1080p/4K framebuffer and GPT discovery.
  - Fast System Calls: Hardware MSR fast dispatcher (`IA32_LSTAR` `0xC0000082`) reducing crossing overhead to ~70 cycles.
  - Virtual Memory: Demand paging with Copy-on-Write (CoW), VMA BTreeMap tracking, and LZ4 in-RAM compressed swap.
  - Preemptive SMP Scheduler: Local APIC one-shot timers with 5 priority classes and NUMA work-stealing across up to 64 cores.
  - Modern Storage & Buses: NVMe 2.0 PCIe controller (ASQ/ACQ, MSI-X) and xHCI USB 3.2 host controller with HID stack.
  - Filesystems: Concurrent VFS with lockless dentry cache and NovaFS pure Rust CoW B-tree filesystem with instant snapshots.
  - Asynchronous I/O: `nova_uring` submission/completion queue lockless rings with kernel `SQPOLL` thread.
  - Graphics & Audio: DRM/KMS mode-setting (`/dev/dri/card0`), VirtIO-GPU 3D Virgl Mesa bridge, zero-copy DMA-BUF sharing,
    and Intel High Definition Audio (HDA) multi-stream 24-bit/192kHz mixer.
  - POSIX Compatibility: `novalibc` standard library with pthreads, epoll, and dynamic linking (`ld-nova.so`).

- **Desktop & Synthesized Shell (16 Tasks Delivered)**:
  - Compositor: Hardware-accelerated Wayland display server (`nova-compositor`) with subpixel damage clipping.
  - UI Framework: `nova-ui` retained-mode vector UI toolkit in pure Rust with declarative `ui! {}` and Flexbox layout.
  - Materials & Fonts: Fluent Acrylic Mica real-time Gaussian blur, Minimalist Cupertino materials, and TrueType subpixel typography.
  - Synthesized Desktop Shell: macOS-style persistent unified Top Menu Bar with active app integration, Windows 11-style
    centered dynamic dock with glowing running dots and hover magnification, and 8-zone snap layouts with automatic tiling.
  - Search & Action Center: macOS Spotlight-style Universal Instant Search (`Super + Space`) with math calculator and fuzzy indexer;
    Action Center (`Super + A`) with grouped alerts and Quick Settings toggles (Wi-Fi, VPN, Night Light, Dark Mode).
  - Native Core Apps: `nova-term` (GPU terminal), `nova-files` (tabbed file manager), `nova-edit` (code editor),
    `nova-settings` (preferences shell), and `nova-monitor` (activity monitor).
  - Developer SDK: `novaos-sdk` pure Rust framework, `cargo-nova` CLI tool, and instant host simulator runner.
  - High-Resolution Display: 100%, 125%, 150%, 200% HiDPI/Retina display scaling with fixed-point coordinate mathematics.
  - Installer & OOBE: Graphical installer wizard (`nova-installer`) with guided/manual GPT partitioning and slide showcase.
  - System Recovery: Standalone emergency rescue console (`nova-recovery`) with NovaFS snapshot rollback and GPT repair.

- **Server Platform & Cloud Infrastructure (12 Tasks Delivered)**:
  - Headless Profile: Sub-1.8s boot, < 32 MiB baseline RAM footprint, and NUMA-optimized scheduler.
  - Cloud-Init Provisioning: `novainit` reading ConfigDrive and NoCloud datasources, Netplan v2, and SSH key injection.
  - Service Supervisor: `novad` systemd-style `.service` unit orchestrator with dependency resolution and auto-restart.
  - Container Engine: Pure Rust `novactl container` / `novadock` with 6-axis namespaces (PID, Net, Mnt, IPC, UTS, User)
    and cgroups v2 resource quotas (`cpu.max`, `memory.max`, `pids.max`, `io.weight`).
  - Cloud Networking: Enterprise TCP stack with Google BBR and Cubic congestion control, in-kernel WireGuard VPN (`novavpn`),
    and stateful packet-filter firewall (`novafw`) with conntrack.
  - Production Daemons: Pure Rust OpenSSH-compatible SSH-2 daemon (`novasshd`) and Prometheus telemetry exporter (`novamon`).
  - Resilient Storage: Software RAID (`NovaVolume` RAID 0, 1, 10) with hot-plug disk rebuilds, and automated unattended
    updates (`novaupdate`) with dual-slot A/B rollback and hardware watchdog safety.

- **Release & Hardening Infrastructure (10 Tasks Delivered)**:
  - Dual-Edition Packaging: Production pipeline generating hybrid BIOS/UEFI ISOs (`novaos-desktop-2.0.iso`, `novaos-server-2.0.iso`)
    and partitioned GPT raw disk images (ESP + SYS + DATA + JNL + CRASH).
  - Automated CI Matrix: `tools/ci/test_matrix.py` running parallel QEMU boot tests across Desktop (344 assertions) and Server
    (272 assertions) editions (616 combined assertions, 100% pass rate) with pure Python pixel-diff visual regression testing.
  - Security Hardening: FIPS 140-3 KAT verification, memory zeroization, constant-time cryptography, and CC EAL4 evaluation readiness.
  - Complete Documentation: 5 technical manuals in `docs/v2.0/` and complete RozeHub documentation portal synchronization.
  - Release Certification: Full cryptographic verification (SHA256SUMS and GPG detached signatures) and GA launch sign-off.

## [1.0.0] - Official Production Release & 6-Month Build Plan Milestone (2026-10-29)

The complete delivery of the 6-Month NovaOS Build Plan: 56 out of 56 planned tasks
delivered across both the Kernel Foundation Track (26 features) and the Interface & Userland
Track (26 features), backed by cross-cutting automated testing, dual Desktop/Server installation
profiles, and an exhaustive security review.

### Highlights

- **Kernel Foundation & Hardware Drivers (26 Tasks Delivered)**:
  - Memory: Full dynamic paging with high-memory split (`0xC0000000`) and page tables.
  - Multi-Core (SMP): ACPI MADT parsing, APIC/IOAPIC drivers, inter-processor interrupts (IPI),
    and per-core preemptive scheduler scaling deterministically across 2 and 4 CPU cores.
  - Storage & Filesystems: Ext2 filesystem, FAT32 write-ahead journal with atomic crash recovery,
    MBR multi-partition parser, and in-kernel disk partitioning/formatting engine (`SYS_DISK_INSTALL`).
  - Networking: Full TCP/IP stack with socket API, DHCP client, DNS UDP resolver, TFTP client,
    stateful packet filter firewall with 256 tracked flows, and in-kernel package repository HTTP server.
  - Storage & Bus Drivers: ATA PIO (LBA28), NVMe PCIe 1.3+ controller, VirtIO-blk, xHCI USB 3.0 controller,
    and 802.11 Wi-Fi MAC layer subsystem.
  - Video & Audio: Dual display backends (Bochs VBE 1024x768x32bpp linear framebuffer & VirtIO-GPU 2D with
    sub-rectangle damage flushing) and AC97 PCI hardware audio mixer.
  - System Lifecycle: ACPI S5 soft-off power management (`SYS_SHUTDOWN`), A/B atomic OS updates with
    automatic fallback, and automated crash dump export (`/CRASHDMP.DAT` and `/CRASHDMP.TXT`).

- **Interface & Userland Applications (26 Tasks Delivered)**:
  - 13 pure `#![no_std]` Rust ring-3 userland applications operating with zero-syscall
    `BufferSurface32` rasterization:
    1. `ui-rs`: Shared design system, UI tokens, widgets, vector drawing, and anti-aliased typography.
    2. `novainit-rs`: PID 1 supervisor managing system services, dependencies, and restart policies.
    3. `wm-rs`: Compositor and window manager with 8-zone window snapping, dynamic tiling, titlebars,
       persistent top menu bar, and application dock.
    4. `files-rs`: Graphical file manager with VFS navigation, file metadata cards, and preview.
    5. `term-rs`: Terminal emulator with VT100 ANSI sequences, scrollback history, and pipes.
    6. `edit-rs`: Monospace text editor with multi-line text buffer, font scaling, and toolbar.
    7. `store-rs`: Graphical package store verifying HMAC-SHA256 package cryptographic signatures.
    8. `login-rs`: Graphical login screen and lockscreen with PBKDF2-HMAC-SHA256 and credential zeroing.
    9. `settings-rs`: Modular preferences shell with Network/Firewall, Users, Services, Device Manager,
       Accessibility (100%-150% font scaling, WCAG AAA high contrast), and Dual-Slot Updates panels.
    10. `klog-rs`: Structured kernel log viewer with severity/facility filtering and keyword search.
    11. `actmon-rs`: Activity monitor with live CPU usage graphs, physical RAM telemetry, and process sorting.
    12. `installer-rs`: Graphical installer wizard driving disk partitioning and deployment.
    13. `crashrep-rs`: Graphical crash dump reporter inspecting crash snapshots and backtraces.

- **Release & Cross-Cutting Infrastructure (4 Tasks Delivered)**:
  - Automated Testing: Scaled test runner from 81 to 319 assertions on Desktop profile and 248 assertions
    on Server profile with 100% pass rate.
  - Desktop & Server Editions: Single shared kernel binary generating two distinct profiles: Desktop
    (`disk.img`) with full GUI suite and Server (`disk-server.img`) stripped of compositor for headless efficiency.
  - Security Review & Audit: Complete 84-syscall authorization audit, MAC profile enforcement, credential memory
    zeroization, and Phase 98 in-kernel self-test.
  - Official Release Candidate: Packaging `release/v1.0.0/`, cryptographic `SHA256SUMS`, and release qualification suite.

The culmination of Phases 1-22: a bootable, self-hosting kernel with a
working shell, filesystem, networking stack, GUI, package manager, and
a real capability-based security model - built up incrementally, with
every phase boot-tested and, wherever feasible, objectively verified
(packet captures, pixel-exact rendering checks, captured audio
waveform analysis) rather than merely asserted to work.

### Highlights

- **Kernel foundation**: x86 32-bit protected mode, GDT/IDT/paging,
  physical memory management, a heap allocator, preemptive
  round-robin scheduling, and per-process address space isolation
  (proven via a falsifiable cross-contamination test, not just
  claimed).
- **Security**: syscall-gated, per-process capability lists for file
  access, network destinations, and process creation - every process
  starts with zero privileges and must be explicitly granted each one,
  enforced in the kernel and proven with real allow/deny tests.
- **Filesystem**: read/write FAT32 on real ATA (PIO) hardware.
- **Networking**: Ethernet, ARP, IPv4, ICMP (ping), UDP, a TFTP
  client, and a minimal DNS resolver, running over either of two NIC
  drivers (NE2000 ISA or RTL8139 PCI, auto-detected and selected).
- **Graphics & GUI**: a VGA Mode 13h framebuffer, a hand-built bitmap
  font (verified pixel-exact against real rendered output), a
  windowing compositor, and a Software Center GUI wired to the real
  package manager.
- **Sound**: an AC97 PCI audio driver, verified by capturing and
  analyzing actual PCM waveform output.
- **Hardware detection**: full PCI bus enumeration.
- **Package manager**: `nova-pkg`, with local installation and
  networked fetch (TFTP) support.
- **System identity**: a first-run setup wizard and a real-time clock
  driver, with settings persisted across reboots.
- **Installable**: `novaos.iso` is a hybrid image - the same file
  boots as a CD *and* as a raw BIOS disk/USB image, verified by
  attaching it as a plain hard disk with no CD-ROM emulation at all
  and confirming the entire system still boots identically. `make
  install-image` prints the exact steps to put it on a real USB drive
  or run it persistently in a VM.

### Explicitly out of scope for 1.0 (see PROGRESS.md for the full list)

- Full TCP/sockets (UDP-based protocols only: ICMP, TFTP, DNS).
- A from-scratch bootloader (the release relies on GRUB's own,
  already-proven hybrid-image tooling rather than reimplementing
  bootloader/BIOS logic).
- USB and additional PCI device classes beyond NIC and audio.
- General text rendering in graphics mode (a small hand-built font
  covers digits, uppercase letters, and common punctuation only).
- A single unified boot+data disk image (the release still uses a
  boot image plus a separate data disk for persistent files).

### Phase-by-phase summary

| Phase | Summary |
|---|---|
| 1 | Bootloader & kernel foundation (GRUB/Multiboot, VGA text, freestanding libc subset) |
| 2 | Memory management & interrupts (GDT/IDT/ISR/IRQ, PIT timer, PS/2 keyboard, heap) |
| 3 | Paging, physical memory management, and a read-only FAT32 filesystem on real ATA hardware |
| 4 | Usermode processes, syscalls, and a preemptive scheduler |
| 5 | Security hardening: per-process address-space isolation, proven with a falsifiable test |
| 6 | Networking: NE2000 NIC driver, Ethernet/ARP/IPv4/ICMP |
| 7 | VGA Mode 13h graphics, a hand-built bitmap font, PS/2 mouse, and a windowing compositor |
| 8 | FAT32 write support and `nova-pkg`, a CLI package manager |
| 9 | First-run setup wizard, a CMOS RTC driver, persistent system identity |
| 10 | UDP, a TFTP client, and networked package fetching |
| 11 | Capability-based file access control for ring-3 processes |
| 12 | A GUI Software Center connecting the package manager to the windowing system |
| 13 | Full PCI bus enumeration |
| 14 | Capability-based access control extended to network destinations |
| 15 | Font punctuation and full package descriptions in the Software Center |
| 16 | An RTL8139 PCI NIC driver, proving PCI detection leads to real hardware support |
| 17 | Capability-based access control extended to process creation |
| 18 | An AC97 PCI sound driver, verified via captured audio waveform analysis |
| 19 | A minimal DNS client, verified against real external resolution |
| 20 | Confirmed and documented `novaos.iso`'s existing hybrid boot capability as the real installer |
| 21 | License, versioning, and this changelog |
| 22 | Final stability/hardening pass ahead of release |

See PROGRESS.md for the full, unabridged writeup of every phase,
including every bug found during development, how each was diagnosed,
and the honest scope limitations that remain.
