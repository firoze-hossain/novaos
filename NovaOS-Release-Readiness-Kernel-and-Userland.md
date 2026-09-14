# NovaOS: Release-Readiness Feature List (Kernel First, Then Userland)

*Updated again: Phases 38-44 are now done (see below) - kernel work
only, matching your stated priority; nothing in Part 2 (userland) has
changed since this document was last updated, so that section is
unmodified. Every "done" mark below reflects verified, shipped work,
not a plan - and where an investigation changed the original claim
itself (the scheduler bug, and which drivers actually polled
continuously), that correction is stated plainly, not glossed over.*

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

- ✅ **Config-driven boot handoff** (Phase 37). The kernel no longer
  hardcodes which program it execs as PID 1 — `SYSTEM.CFG`'s
  `init_path` field decides, defaulting to `SHELL.ELF`. Proven working
  end to end: the exact same kernel binary was booted against a second
  disk image with `init_path` pointed at `HELLO.ELF` instead, and it
  launched that program instead. The concrete prerequisite for "one
  kernel, multiple distros."
- ✅ **Pipes / first real IPC** (Phase 36). Processes can write to and
  read from each other via a real kernel primitive, not just files.
  Known limit: not inherited across `fork()` yet, so a genuine shell
  `|` between two independent processes isn't reachable yet either.
- ✅ **A real, live correctness bug found and fixed** (Phase 38) — see
  1.3 below for the honest account of what it actually was (not what
  it first looked like).
- ✅ **Driver self-registration** (Phase 39) — see 1.2 below for exact
  scope (4 of this kernel's ~10 drivers migrated so far, not all).
- ✅ **Kernel synchronization primitives** (Phase 40) — `SpinLock<T>`
  now exists and is applied to real, already-shipped shared state. See
  1.4 below for what this does and doesn't mean for SMP itself.
- ✅ **virtio-blk** (Phase 42) — this kernel's first virtio driver. See
  1.2 below for exact scope (legacy transport only, not VFS-mounted).
- ✅ **RTL8139 genuinely interrupt-driven** (Phase 43) — see 1.2 below
  for the honest correction to the original "every driver polls" claim
  this same document made.
- ✅ **ACPI CPU topology discovery** (Phase 44) — see 1.4 below; this
  is the first SMP prerequisite, not SMP itself.

---

## Part 1 — Kernel: what's needed to release

### 1.1 Real users and permissions

| What | Why a user notices | Best-in-class reference |
|---|---|---|
| **UID/GID, login, file ownership.** Currently zero permission bits exist anywhere in the kernel — no `chmod`-equivalent, no concept of "who owns this file or process" at all. | Without this, there's no real notion of privacy or protection between accounts — a dealbreaker the moment more than one person (or even one person with an admin vs. normal-use account) touches the machine. | **Linux/Unix's UID/GID + permission-bits model** — simple, proven, and the foundation everything else (sudo, ACLs, containers) builds on. Start here, not with something more elaborate. |
| **A real login screen / session concept**, distinct from the current cosmetic "username" set once at first boot. | The single most visible "is this a real OS" signal to a new user. | **macOS's login window** is the cleanest reference for a hobby OS — simple, fast, no unnecessary complexity. |
| **A privilege-escalation model** (`sudo`-equivalent) once real users exist. | Lets normal use stay unprivileged by default — a genuine security property, not just cosmetic. | **Linux's `sudo`** (simpler to implement than Windows' UAC prompt-and-consent flow, and gets you 90% of the value). |

### 1.2 Drivers that don't require editing kernel boot code

| What | Status | Why a user notices | Best-in-class reference |
|---|---|---|---|
| **A driver registration table.** | ✅ **Done (Phase 39), partial scope.** `DRIVER_REGISTER(name, init_fn, phase)` places a driver into a dedicated linker section; `kernel_main()` no longer calls driver init functions by name for the ones migrated. Proven, not just claimed: a temporary demo driver was added in its own new file and confirmed to run with `kernel/init/main.c` completely unchanged. **4 of this kernel's ~10 drivers are migrated so far** — PS/2 keyboard, PS/2 mouse, UHCI, AC97. `timer_init`/`vfs_init`/`net_init` remain explicit calls, deliberately: each is interleaved with its own self-tests immediately after, and migrating those needs a real design decision (does the self-test move with it, or become separate?), not just mechanical copying. | Directly blocks "this OS works on more than the exact hardware it was tested against" — the single biggest thing standing between "runs in QEMU" and "runs on a real, different machine." | **Windows NT's HAL** is the textbook reference — it's *the* reason NT's kernel source ported across x86/MIPS/Alpha/PowerPC/ARM unchanged. |
| **virtio-net / virtio-blk drivers.** | ✅/❌ **virtio-blk done (Phase 42), legacy transport, real hardware DMA verified. virtio-net not started.** The virtqueue itself (descriptor/available/used rings) is in Rust, matching the same "ring buffer, index arithmetic must never be off by one" reasoning as this kernel's own pipes. Proven against real QEMU hardware: a write-then-read-back of an actual 512-byte sector through real DMA succeeded with a 256-entry queue. **Not wired into the VFS** — FAT32/ext2 still mount through the existing ATA driver; a real virtio-blk-backed filesystem is separate, larger follow-up work (which device wins if both ATA and virtio-blk are present is a real, unanswered design question). | Directly relevant since you develop under QEMU — virtio is the standard, dramatically simpler way a VM talks to its host. | **Linux popularized virtio**; it's now the universal VM driver standard across every hypervisor. |
| **IRQ-driven drivers, not polling.** | ⚠️ **Partially done (Phase 43) — and the original claim in this row turned out to be only partly accurate.** Direct investigation found `ac97_beep()` is actually fire-and-forget (no poll loop at all after starting playback) and UHCI's only busy-wait is bounded to one-time enumeration at boot — neither has the "wastes CPU continuously while idle" property this row originally claimed for *every* driver. The pattern that genuinely did match: `idle_task_entry()` called `net_poll()` on every timer tick forever, reading RTL8139's hardware register whether or not a packet had arrived. **That specific pattern is now fixed** — RTL8139 has a real IRQ handler (PCI Interrupt Line register, not hardcoded), and `net_poll()` touches no hardware at all when nothing's pending. Verified with a real, unambiguous measurement, not inference: a temporary counter showed the handler fired 12 times during one test boot. NE2000 (not exercised by this project's own test config) and RTL8139's own TX path are untouched. | Wastes CPU continuously, even when a real user is doing nothing — shows up as fan noise / battery drain / sluggishness on real hardware in a way it never does in QEMU. | **All three (Linux/Windows/macOS)** treat polling drivers as the exception, not the rule. |

### 1.3 Won't crash, won't corrupt data, tells you why when it fails

| What | Status | Why a user notices | Best-in-class reference |
|---|---|---|---|
| **Fix the live, confirmed scheduler/`process_wait()` bug** found during Phase 36. | ✅ **Fixed (Phase 38) — and it wasn't the scheduler at all.** The real root cause, found by instrumenting the scheduler directly and tracing a real page-fault → double-fault → triple-fault cascade with QEMU's own exception tracing: `tools/linker.ld` didn't capture rustc's per-symbol section naming, leaving a Rust static's memory outside the range the kernel reserves as "already mine" — the physical memory manager then handed that exact memory to an unrelated process's page directory, which silently aliased the kernel's own live data. Confirmed directly: decoded the corrupted page-directory bytes back to the exact ASCII text of the colliding data. Fixed generally (matching both plain and per-symbol section names for every section type), not as a one-off patch. | A hang with no error message is the single worst experience an OS can produce. | N/A — this was a correctness bug, not a missing feature. |
| **A journaled or copy-on-write filesystem.** | ❌ Not started. ext2 is still read-only; FAT32 still has no journaling. | Power loss mid-write currently risks real data corruption. | **macOS's APFS** or **Linux's ext4/btrfs journaling**. |
| **A structured crash-dump, not just a serial-port panic log.** | ❌ Not started. | The difference between "I can tell you exactly what broke" and "the machine just stopped." | **Windows' minidump format**. |
| **Real shutdown, not just reboot.** | ❌ Still not done, but the story changed: **ACPI parsing now exists** (Phase 44) — this kernel can read and validate real ACPI tables (RSDP/RSDT/MADT), which it genuinely couldn't before. That parsing is currently scoped to CPU topology discovery (see 1.4), not power management — real shutdown needs a different table (the FADT) and its `PM1a_CNT` register, which this kernel doesn't read yet. The foundation (real, verified ACPI table parsing) is no longer the blocker it was; the specific shutdown piece is still unbuilt. | An OS that can't turn the computer off doesn't feel finished. | Baseline expectation across all three. |

### 1.4 Multi-core

| What | Status | Why a user notices | Best-in-class reference |
|---|---|---|---|
| **Kernel synchronization primitives.** | ✅ **Done (Phase 40).** `SpinLock<T>` — the `spin_lock_irqsave`/`spin_unlock_irqrestore` shape (disables local interrupts while held *and* spins on a real atomic, so it's correct even once a second CPU exists, not just today). Applied to real, already-shipped state (`kernel/rust/pipe.rs`'s own `PIPES` table), not left as an unused primitive. This specifically closes the "no spinlock/mutex exists yet" half of the original claim in this row. | A genuine prerequisite for everything below — not something that mattered for its own sake until now. | **Linux's own approach** (fine-grained locking) is the long-term reference; this project's `SpinLock` is deliberately much simpler, correct for what it protects today. |
| **CPU topology discovery (ACPI/MADT parsing).** | ✅ **Done (Phase 44) — the genuine first SMP prerequisite, verified three separate ways** including three independent real-hardware data points (`-smp 1/2/4`, each logged count exactly matching). A real bug (a page fault reading real ACPI tables placed outside this kernel's identity-mapped range) was found and fixed correctly, not papered over. **This is not SMP** — nothing about how this kernel boots, schedules, or handles interrupts changed; no second CPU is started. | The literal first fact any SMP implementation needs (how many CPUs, and their APIC IDs) — previously not knowable at all. | Every real OS (Linux, Windows, macOS) does exactly this, this way, first. |
| **SMP support itself.** | ❌ **Still not started — substantial, separate work, deliberately not rushed.** What's still needed, in roughly the order it's needed: a Local APIC driver (replacing/supplementing the 8259 PIC this kernel's *entire* interrupt architecture currently runs on); an IO-APIC driver; an AP (secondary CPU) bootstrap trampoline living in low memory; per-CPU data structures (current process, kernel stack, TSS, one set per CPU instead of one global set); a scheduler capable of running on more than one CPU without two CPUs ever picking the same process; and — the genuinely hardest part, confirmed by this project's own experience building just the two primitives above — auditing and locking *every* existing shared kernel structure (`process_table[]`, `open_files[]`, the PMM bitmap, the heap allocator, every driver's own state) for real multi-CPU safety. `SpinLock` gives the *tool*; applying it correctly everywhere it's needed is the actual, much larger remaining task. | Every machine sold in the last 15+ years has multiple cores. An OS that uses one is leaving most of the hardware's actual performance on the table, visibly. | All three do this; Linux's own multi-year effort at fine-grained locking is the concrete, hardest-earned lesson that this remains real, unrushed work, not a checkbox. |

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
- ~~Fix the confirmed scheduler/`process_wait()` bug (Part 1.3)~~ ✅
  **done (Phase 38)** — turned out to be a linker script bug, not the
  scheduler; see 1.3 for the corrected account.
- Real users/permissions, kernel side (Part 1.1) — still not started.
- ~~Driver registration table (Part 1.2)~~ ✅ **done (Phase 39)**,
  partial scope (4 of ~10 drivers migrated) — extending to the
  remaining drivers (`timer`/`vfs`/`net`) is real, smaller follow-up
  work, not a new milestone item.
- Remaining coreutils ported to ring-3 (Part 2.1) — still not started;
  see Part 2, unchanged since this document was last updated.

**v1.0 ("I'd hand this to a curious friend")**
- Login screen + `sudo`-equivalent (Part 1.1) — still not started.
- Dynamic linking (Part 2.4) — still not started.
- Package management with real network fetch (Part 2.2) — still not
  started.
- A window manager with actual dragging/management (Part 2.3) — still
  not started.
- SMP support (Part 1.4) — **still not started, and still belongs
  here, not earlier.** Its two real prerequisites (synchronization
  primitives, CPU discovery) are now done (Phases 40, 44), but SMP
  itself - a Local/IO-APIC driver, AP bootstrap, per-CPU state, and
  auditing every existing shared kernel structure for multi-CPU safety
  - remains substantial, deliberately unrushed work. Having the
  prerequisites done makes this less of a blank slate than it was, not
  less of an undertaking.

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
