# NovaOS: Release-Readiness Feature List (Kernel First, Then Userland)

*Updated version of the earlier gap-analysis doc. What changed:
Phase 37 (config-driven boot handoff — the kernel no longer hardcodes
`"SHELL.ELF"`) is now done and marked as such below. This version is
reorganized around your latest ask specifically: not just "what's
missing," but "what's the major feature list to make this genuinely
usable and release-worthy for a real user," kernel first, then
userland — with the best ideas from macOS, Linux, and Windows folded
directly into each relevant item instead of siloed in their own
section (there's still a quick-reference summary by OS at the end).*

---

## How to read this

- **Part 1 — Kernel.** Your stated first priority. Organized as a
  checklist: what it is, why a *real user* would notice its absence,
  and which OS does it best (worth studying as a concrete reference
  when you build it).
- **Part 2 — Userland.** Same format, second priority.
- **Part 3 — Quick reference.** If you only remember one thing to
  borrow from each OS, this is it.
- **Part 4 — Suggested release milestones.** A rough v0.x → v1.0 → v2.0
  shape, so "release" has a concrete meaning rather than being an
  open-ended goal.

Every kernel/userland fact below is grounded in the actual current
source, not assumed from a phase number.

---

## What's already done (don't re-solve these)

- ✅ **Config-driven boot handoff** (Phase 37, just shipped). The
  kernel no longer hardcodes which program it execs as PID 1 —
  `SYSTEM.CFG`'s `init_path` field decides, defaulting to `SHELL.ELF`.
  Proven working end to end: the exact same kernel binary was booted
  against a second disk image with `init_path` pointed at `HELLO.ELF`
  instead, and it launched that program instead — confirmed via serial
  log, not just inferred from the diff. This is the concrete
  prerequisite for "one kernel, multiple distros" from your earlier
  question, and it's real now.
- ✅ **Pipes / first real IPC** (Phase 36, implemented and verified,
  pending your push). Processes can now write to and read from each
  other via a real kernel primitive, not just files.

---

## Part 1 — Kernel: what's needed to release

### 1.1 Real users and permissions

| What | Why a user notices | Best-in-class reference |
|---|---|---|
| **UID/GID, login, file ownership.** Currently zero permission bits exist anywhere in the kernel — no `chmod`-equivalent, no concept of "who owns this file or process" at all. | Without this, there's no real notion of privacy or protection between accounts — a dealbreaker the moment more than one person (or even one person with an admin vs. normal-use account) touches the machine. | **Linux/Unix's UID/GID + permission-bits model** — simple, proven, and the foundation everything else (sudo, ACLs, containers) builds on. Start here, not with something more elaborate. |
| **A real login screen / session concept**, distinct from the current cosmetic "username" set once at first boot. | The single most visible "is this a real OS" signal to a new user. | **macOS's login window** is the cleanest reference for a hobby OS — simple, fast, no unnecessary complexity. |
| **A privilege-escalation model** (`sudo`-equivalent) once real users exist. | Lets normal use stay unprivileged by default — a genuine security property, not just cosmetic. | **Linux's `sudo`** (simpler to implement than Windows' UAC prompt-and-consent flow, and gets you 90% of the value). |

### 1.2 Drivers that don't require editing kernel boot code

| What | Why a user notices | Best-in-class reference |
|---|---|---|
| **A driver registration table.** Right now every driver's init is a hardcoded function call inside `kernel_main()`. Adding hardware support means editing kernel source, not installing a driver. | Directly blocks "this OS works on more than the exact hardware it was tested against" — the single biggest thing standing between "runs in QEMU" and "runs on a real, different machine." | **Windows NT's HAL (Hardware Abstraction Layer)** is the textbook reference — it's *the* reason NT's kernel source ported across x86/MIPS/Alpha/PowerPC/ARM unchanged. Worth studying its exact boundary even at a much smaller scale. |
| **virtio-net / virtio-blk drivers.** | Directly relevant since you develop under QEMU — virtio is the standard, dramatically simpler way a VM talks to its host, vs. emulating real NE2000/RTL8139/ATA hardware. Also faster boot, better throughput. | **Linux popularized virtio**; it's now the universal VM driver standard across every hypervisor. |
| **IRQ-driven drivers, not polling.** Every current driver (NE2000, RTL8139, AC97, UHCI) polls instead of using interrupts. | Wastes CPU continuously, even when a real user is doing nothing — shows up as fan noise / battery drain / sluggishness on real hardware in a way it never does in QEMU. | **All three (Linux/Windows/macOS)** treat polling drivers as the exception, not the rule — this is baseline, not a "nice to have." |

### 1.3 Won't crash, won't corrupt data, tells you why when it fails

| What | Why a user notices | Best-in-class reference |
|---|---|---|
| **Fix the live, confirmed scheduler/`process_wait()` bug** found during Phase 36 — a parent process occasionally never resumes after an unrelated child exits. This is real, reproduced, and still open. | A hang with no error message is the single worst experience an OS can produce. This should be the actual next thing worked on, ahead of new features. | N/A — this is a correctness bug, not a missing feature. Fix it first. |
| **A journaled or copy-on-write filesystem** (ext2 is currently read-only; FAT32 has no journaling at all). | Power loss mid-write currently risks real data corruption — every mainstream OS filesystem exists specifically to prevent this. | **macOS's APFS** (copy-on-write, instant snapshots, crash-safe by design) is the more modern reference; **Linux's ext4/btrfs journaling** is the more battle-tested, simpler-to-implement one. |
| **A structured crash-dump, not just a serial-port panic log.** | The difference between "I can tell you exactly what broke" and "the machine just stopped" once you're debugging on real hardware without a serial cable attached. | **Windows' minidump format** — small, structured, designed specifically to be mailed to a developer and opened without the original machine. |
| **Real shutdown, not just reboot.** No ACPI parsing exists yet — the kernel can restart but can't power off. | An OS that can't turn the computer off doesn't feel finished, full stop. | Baseline expectation across all three — this is table stakes, not a differentiator. |

### 1.4 Multi-core

| What | Why a user notices | Best-in-class reference |
|---|---|---|
| **SMP support.** Single-core only today; no synchronization primitives (no spinlock/mutex anywhere in the kernel) exist yet either — a genuine prerequisite, not parallel work. | Every machine sold in the last 15+ years has multiple cores. An OS that uses one is leaving most of the hardware's actual performance on the table, visibly. | All three do this; **Linux's approach (fine-grained locking evolved over decades)** is the hardest-earned lesson here — worth knowing it took Linux itself many years to get right, so budget accordingly rather than rushing it. |

### 1.5 A real network stack

| What | Why a user notices | Best-in-class reference |
|---|---|---|
| **TCP retransmission/windowing** (currently stop-and-wait only) and a **sockets-style syscall API** exposed to ring-3 (TCP is currently C-function-call-only, not reachable from userland at all). | Anything beyond a local virtual link currently fails silently. No ring-3 program can use TCP today at all — this blocks a real browser, a real download manager, anything network-facing in userland. | **Linux's Berkeley sockets API** (`socket()`/`bind()`/`connect()`) is *the* universal reference every other OS's networking API is compatible with or inspired by — implement this shape specifically, not something novel. |

---

## Part 2 — Userland: what's needed to release

### 2.1 A shell and coreutils that feel complete

| What | Why a user notices | Best-in-class reference |
|---|---|---|
| Port the remaining coreutils to ring-3 (`ls`, `echo`, `cp`, and others exist only in the legacy ring-0 shell today — only `cat` is a real ring-3 utility so far). | The everyday, moment-to-moment experience of using the shell — this is what "the OS feels finished" mostly *is*, in practice. | **Linux/BSD coreutils** — the exact, well-known command set users already expect. Don't invent new names/flags; match the familiar ones. |
| `ping`/`nslookup`/`tftp`/`pkg`/the GUI aren't reachable from the ring-3 shell yet. | Right now the "real" shell (ring-3) is meaningfully less capable than the old ring-0 one — a visible regression to anyone who's used both. | — |

### 2.2 Package management that actually installs software

| What | Why a user notices | Best-in-class reference |
|---|---|---|
| **Network fetch for packages** (no HTTP/FTP in the network stack yet — `pkg` can only install from what's already on disk). | "Install new software" is one of the top things any OS user expects to do. Without network fetch, this isn't really package management yet, just local file management. | **APT (Debian/Ubuntu)** for the simplest, most proven end-to-end shape: a repository format, dependency resolution, signed packages. Don't reinvent this — copy the shape. |
| **Package signing / verification.** No code-signing of any kind exists yet — any ELF that parses, runs. | Directly a security gap: nothing stops a corrupted or malicious package from running silently. | **macOS's Gatekeeper + notarization**, or **Windows' Authenticode** — either is a fine reference; the core idea (verify a signature before trusting a binary) is what matters, not the exact mechanism. |

### 2.3 A desktop that's actually usable, not just a proof of concept

| What | Why a user notices | Best-in-class reference |
|---|---|---|
| **Real window management** — the current compositor is ring-0 only, and the ring-3 graphics syscalls only support a static, non-interactive scene (no dragging, no window management at all). | This is the single most visually obvious "is this a real OS" signal for anyone who isn't a developer. | **macOS's window manager** for polish/feel; **Windows' taskbar + window snapping** for practical, everyday usability wins that are cheap to implement and immediately felt. |
| **A real init/service-management system** (there's none currently — no service supervision, no restart-on-crash, no dependency ordering). | Directly enables "background things just work" — a crashed background service currently just stays crashed, silently. | **macOS's `launchd`** — one daemon replacing init scripts, cron, and on-demand service starting, all from simple declarative config. The cleanest, most approachable reference of the three for a project this size. |

### 2.4 A C library and toolchain solid enough to build real software on

| What | Why a user notices | Best-in-class reference |
|---|---|---|
| **Dynamic linking** (currently none — `PT_DYNAMIC` is explicitly skipped by the ELF loader, every program is static). | Blocks shared libraries entirely — every program duplicates its own copy of every library it uses, which doesn't scale as real software accumulates. | **Linux's ELF dynamic linking (`ld.so`)** — the standard shape; not worth inventing something different. |
| **`realloc`, file streams (`fopen`/`fread`), `errno`, environment variables** — all currently missing from the C library. | These are baseline expectations for *any* C program that isn't written specifically for NovaOS — currently, porting an existing, ordinary C program to NovaOS would fail immediately on missing libc functions. | Standard POSIX libc surface — again, match the existing standard rather than designing a new one. |

---

## Part 3 — Quick reference: the one best idea per OS

If you remember nothing else from this document:

- **From Linux:** the **UID/GID + permission-bits model**, and the
  **Berkeley sockets API**. Both are the "don't reinvent this, just
  implement the well-known shape" category — decades of proof this is
  the right design, and every other OS's equivalent is either
  compatible with or directly descended from these two.
- **From Windows (NT):** the **HAL (Hardware Abstraction Layer)**.
  Directly the fix for "editing kernel source to add a driver" — study
  NT's exact boundary between hardware-specific and hardware-
  independent kernel code as a concrete design reference.
- **From macOS:** **`launchd`**. The cleanest, most approachable
  reference for "one thing manages services/startup," and a natural
  fit for a project at NovaOS's current scale — simpler to reason
  about than systemd's larger surface, while solving the same real
  problem.

---

## Part 4 — Suggested release milestones

A rough shape, so "release" means something concrete:

**v0.x (where you are now → real usability threshold)**
- Fix the confirmed scheduler/`process_wait()` bug (Part 1.3) — before
  anything else.
- Real users/permissions, kernel side (Part 1.1).
- Driver registration table (Part 1.2) — unlocks adding hardware
  support without kernel surgery.
- Remaining coreutils ported to ring-3 (Part 2.1).

**v1.0 ("I'd hand this to a curious friend")**
- Login screen + `sudo`-equivalent (Part 1.1).
- Dynamic linking (Part 2.4).
- Package management with real network fetch (Part 2.2).
- A window manager with actual dragging/management (Part 2.3).
- SMP support (Part 1.4) — real modern-hardware performance.

**v2.0 ("I'd hand this to someone who isn't a developer")**
- Journaled/COW filesystem (Part 1.3).
- Package signing (Part 2.2).
- `launchd`-style service management (Part 2.3).
- Structured crash dumps (Part 1.3).
- Sockets API + TCP retransmission (Part 1.5) — real networking
  reachable from real userland software.

Everything else in Parts 1–2 makes NovaOS better at any point along
this path — this is a suggested *order*, not a claim that anything
outside it doesn't matter.
