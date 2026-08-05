# NovaOS: Roadmap Beyond v1.0

## Read this first

You asked for a complete plan to bring NovaOS to "Linux, Windows, and
macOS level, with all functionality." I want to be straight with you
rather than hand you a phase list that implies that's achievable
through more iterative phases, because it isn't - and you deserve an
honest answer, not a flattering one.

**Scale comparison:**

| | Lines of code | Engineers | Time |
|---|---|---|---|
| Linux kernel alone | ~30 million | Thousands of contributors | 30+ years |
| Windows (NT kernel + userland) | Tens of millions | Tens of thousands | 30+ years |
| macOS / Darwin | Tens of millions (much inherited from BSD/Mach) | Thousands | 20+ years on top of decades of prior Unix work |
| **NovaOS today (v1.0)** | **~15,000 lines** | **You + iterative AI-assisted development** | **A few months** |

Those three systems also include things no phase-based roadmap can
shortcut regardless of how well it's planned:

- **Certified hardware driver ecosystems** covering tens of thousands
  of real devices (GPUs, WiFi chipsets, laptop power management ASICs,
  printers, etc.) - each vendor-specific, many requiring signed NDAs
  with hardware vendors to get register documentation at all.
- **GPU-accelerated graphics** - modern desktops render via
  vendor GPU drivers (NVIDIA/AMD/Intel) talking to hardware through
  proprietary or semi-open command submission protocols, then a
  compositor (Wayland, DWM, Quartz) built on that. This alone is a
  multi-year undertaking even for one vendor's hardware.
- **Decades of API surface** - POSIX (Linux/macOS) and Win32/COM/.NET
  (Windows) each represent thousands of documented functions that
  millions of existing programs depend on being bit-for-bit correct.
- **Application ecosystems** - browsers, office suites, IDEs, games -
  representing, cumulatively, more engineering effort than the
  operating systems themselves.
- **Security audit trails and certifications** (Common Criteria, FIPS,
  decades of CVE response process) that enterprises and governments
  require before deployment.

None of this means NovaOS's development has been a toy exercise - the
opposite: every phase so far has been genuinely real, tested code
solving genuinely hard problems (a working scheduler, real memory
isolation, real network drivers, a real filesystem). But "genuinely
real hobby/educational OS with a lot of working subsystems" and
"competes with Linux/Windows/macOS" are different categories of
project, the way a well-built go-kart and a Formula 1 car are both
genuinely real vehicles that share a category without being
comparable in scope.

**What this document actually gives you**: an honest, tiered roadmap
of where NovaOS can realistically go next, with the far end explicitly
marked as "aspirational, likely years of dedicated team effort" rather
than something the next 10 phases produce. Tier 2 is achievable in the
same iterative style used for Phases 1-22. Tier 3 is a genuine
multi-year undertaking even for a dedicated small team. Tier 4 is
included for completeness and honesty about what "Linux/Windows/macOS
level" actually requires - not as a near-term target.

---

## Where NovaOS stands today (v1.0, Phases 1-22)

A working, boot-tested, professionally-documented hobby/educational
OS with:

- x86 32-bit protected-mode kernel: GDT/IDT/paging, physical memory
  management, a heap allocator, preemptive scheduling
- Per-process address space isolation, proven with a falsifiable test
- Syscall-gated capability-based security for files, network, and
  process creation - genuinely novel among hobby OS projects at this
  scale
- Read/write FAT32 on real ATA hardware
- A real network stack: Ethernet, ARP, IPv4, ICMP, UDP, TFTP, DNS,
  over either of two NIC drivers (NE2000 ISA, RTL8139 PCI)
- VGA graphics, a hand-built font, a windowing compositor, a GUI
  package manager
- An AC97 sound driver, verified via captured waveform analysis
- Full PCI bus enumeration
- A package manager with local + networked installation
- A genuinely installable disk image (GRUB hybrid boot, verified to
  boot as a real BIOS disk/USB image)
- MIT licensed, versioned, changelogged

This is a strong foundation - comparable in scope to well-regarded
teaching operating systems (MIT's xv6, the various OSDev.org
community projects) and in some areas (the capability-security model,
the breadth of verified-not-assumed driver work) more thorough than
many of them.

---

## Tier 2: A capable, modern hobby/teaching OS

*Realistic scope for continued iterative development, the same style
as Phases 1-22. Each item below is roughly Phase-sized (one to a few
sessions of focused work) and would be genuinely valuable progress -
not filler.*

### 2.1 Core architecture
- **x86_64 (64-bit) port** - a substantial rewrite: 4-level page
  tables, a new calling convention, GDT/IDT changes, larger address
  space. Arguably the single highest-leverage item in this tier, since
  it's a prerequisite for running most modern compiled software.
- **SMP (multi-core) support** - per-CPU data structures, spinlocks,
  inter-processor interrupts (IPIs), a real scheduler lock. Currently
  NovaOS assumes one CPU throughout.
- **Demand paging + copy-on-write** - currently all memory is
  eagerly allocated; real virtual memory (page faults triggering
  on-demand allocation, COW for `fork()`-like semantics) is
  foundational for everything in 2.2.

### 2.2 Process model maturity
- **ELF loading** - load and execute real compiled programs from
  disk, not just C functions compiled into the kernel image. This is
  the single most important unlock for "running real software" and a
  natural next phase given FAT32 write support already exists.
- **`fork()`/`exec()`-style process semantics** - real Unix-like
  process creation, replacing the current fixed-menu `SYS_SPAWN`.
- **Signals** - a basic signal delivery mechanism (SIGKILL-equivalent
  at minimum).
- **A real libc** - enough of a C standard library (malloc, string
  functions, basic I/O) linked into user programs to make ELF loading
  actually useful, likely by porting a minimal existing libc (newlib,
  musl) rather than writing one from scratch.
- **Dynamic linking** - shared library loading, needed once more than
  trivial programs are being run.

### 2.3 Filesystem
- **A real filesystem beyond FAT32** - ext2 is the natural next step
  (well-documented, simpler than ext4/btrfs, ubiquitous reference
  implementations to check against). Adds permissions, symlinks, and
  no 4GB file-size ceiling.
- **MBR/GPT partition table support** - directly closes the "still two
  separate disk images" limitation from Phase 20; also a prerequisite
  for a unified boot+data disk.
- **A real VFS layer** supporting multiple filesystem types mounted
  simultaneously (currently FAT32 is hardcoded as "the" filesystem).
- **A block/buffer cache** - currently every read/write hits the disk
  directly; real performance needs a cache layer.

### 2.4 Networking
- **Full TCP** - the state machine, retransmission, and basic
  congestion control this project has deliberately deferred three
  times now (Phases 6, 10, 19) for good risk-management reasons. With
  ELF loading and a libc in place, this becomes much more valuable,
  since it unlocks porting real network software.
- **A BSD-socket-style API** - `socket()`/`bind()`/`connect()`/
  `send()`/`recv()` as syscalls, replacing the current bespoke
  `SYS_NET_SEND`.
- **DHCP client** - currently NovaOS uses a fixed static IP.

### 2.5 Drivers
- **AHCI/SATA** - the current ATA driver is legacy PIO/IDE only; most
  real hardware since ~2010 is SATA via AHCI.
- **USB (UHCI or xHCI, then mass storage + HID device classes)** - a
  genuinely large undertaking on its own, likely 2-3 phases: the host
  controller driver, then USB mass storage (a real path to booting
  from/using USB drives beyond the CD/dd-image approach), then a USB
  keyboard/mouse to replace PS/2 dependence on older chipsets.
- **A real block device abstraction** layer so filesystems don't talk
  directly to ATA-specific code (needed to support both IDE and AHCI
  cleanly).

### 2.6 GUI
- **A real bitmap font covering the full printable ASCII range** -
  the current font deliberately covers only digits, uppercase, and a
  handful of punctuation, verified pixel-exact but narrow in scope.
  Full coverage (including true lowercase forms) would need careful,
  slow, one-glyph-at-a-time verification the same way Phases 7/12/15
  did, not a bulk transcription.
- **A higher resolution / linear framebuffer mode** via VBE, now that
  a full font would make general text rendering viable - Mode 13h's
  320x200 was deliberately chosen in Phase 7 specifically to avoid
  this risk before a real font existed.
- **Window decorations, focus/z-order, and a taskbar** - the current
  compositor (Phase 7/12) is a fixed 3-window demo with no window
  management concepts beyond drag-to-move.

### 2.7 Security
- **Extend capabilities to remaining resource types** - the pattern
  (Phases 11/14/17) generalizes to device access, IPC, and more; each
  extension is a well-scoped, proven-pattern phase.
- **A real multi-user model** - UIDs, a password/login mechanism,
  per-file ownership (meaningful once ext2-style permissions exist).
- **Process resource limits** (memory caps, CPU quotas).

### 2.8 Self-hosting
- **A toolchain that runs on NovaOS itself** - the long-term test of
  "is this a real OS": can it compile and run its own compiler? This
  requires most of 2.1-2.4 as prerequisites and represents a genuine
  capstone milestone for Tier 2, likely 6-12 months of continued work
  at this project's pace, not a handful of phases.

**Honest time estimate for all of Tier 2**: at the pace this project
has moved (roughly one to three phases per working session), Tier 2
represents on the order of 40-80 more phases - a large but genuinely
achievable continuation of exactly what's been happening, likely
spanning many months of continued sessions. Not a short list, but a
real, groundable one.

---

## Tier 3: A self-hosting desktop OS capable of running real software

*A genuine multi-year undertaking even for a small dedicated team,
not a natural continuation of the phase-by-phase pace above. Included
so the roadmap is honest about what comes after Tier 2, not because
it's a near-term plan.*

- **GPU-accelerated graphics** - a real display driver for at least
  one GPU family (likely a simple virtual GPU like QEMU's virtio-gpu
  or bochs-vbe first, real hardware vendors much later), plus a 2D/3D
  acceleration API.
- **A real display server / compositor protocol** (Wayland-like),
  replacing the current single-process compositor model.
- **Porting an existing userland** - coreutils-equivalent tools, a
  real shell beyond NovaOS's built-in one, a text editor, enough of a
  POSIX surface that unmodified simple Unix programs compile and run.
- **Package ecosystem infrastructure** - a real remote repository
  format and index (not just single files fetched by name), dependency
  resolution, signing/verification.
- **Full TCP/IP stack maturity** - IPv6, a proper routing table,
  firewall/filtering.
- **WiFi** - realistically the single hardest driver category to
  reach here, given how poorly-documented and vendor-locked most WiFi
  chipsets are even for established open-source OS projects.
- **Power management (ACPI)** - suspend/resume, battery status,
  thermal management - all currently entirely absent.
- **Audio mixing and a real sound server** (currently one process can
  use the AC97 driver directly; no concept of multiple applications
  sharing audio output).

---

## Tier 4: Feature parity with Linux/Windows/macOS

Included for completeness, not as a plan. This tier is the sum of:
everything in Tiers 2 and 3, PLUS certified drivers for the long tail
of real-world hardware, GPU vendor driver relationships, a security
audit and CVE response process, accessibility infrastructure,
internationalization, enterprise management features (domain
join/group policy equivalents), virtualization host support, and
the ability to run the existing application ecosystems those three
platforms have accumulated over decades. This is realistically a
"large company, many years, large team" scope of effort - not
something any roadmap document should present as a near-term
deliverable, because it isn't one.

---

## What I'd actually recommend

Treat **Tier 2** as the real backlog - it's honestly scoped, matches
how this project has successfully worked so far, and each item
would be a genuinely valuable, testable, professionally-documented
phase exactly like Phases 1-22. Within Tier 2, if you want a concrete
"what's next," I'd prioritize in roughly this order, since each
unlocks the next:

1. **ELF loading** (2.2) - the highest-leverage single item; almost
   nothing else in Tier 2 is as valuable on its own.
2. **A minimal libc port** (2.2) - makes ELF loading actually useful
   for more than trivial programs.
3. **MBR/GPT partition support** (2.3) - directly fixes the
   already-documented "two separate disk images" limitation and is a
   quick, well-scoped win.
4. **Demand paging** (2.1) - needed before `fork()`-style semantics
   make sense.
5. **Full TCP + a real sockets API** (2.4) - now genuinely valuable
   once real programs can run.

I'm glad to keep going and execute these the same way every phase so
far has been done - implemented, boot-tested, verified with real
evidence (not just claimed), documented honestly including whatever
bugs turn up along the way, and delivered as a clean patch. Just say
which one(s) to start on, or "continue with the recommended order"
and I'll pick up at ELF loading.
