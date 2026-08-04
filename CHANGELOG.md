# Changelog

All notable changes to NovaOS are documented here. See
[PROGRESS.md](PROGRESS.md) for full technical detail, verification
evidence, and known limitations behind every phase summarized below.

## [1.0.0] - First Release

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
