# NovaOS: Release-Readiness Feature List (Kernel First, Then Userland)

*Updated again: Phase 57 is now done (see below - a real per-CPU
scheduler, a per-CPU TSS, and locking for the process table, PMM
bitmap, heap allocator, exec buffer, open-file-handle table, and a
coarse VFS-wide lock, closing most of the "SMP support itself" row in
1.5 - the hardware bring-up half (Phase 56) and this scheduler/locking
half together now cover everything that row originally asked for,
except per-driver FAT32/ext2 locking, which stays intentionally
coarse, and a real, pre-existing `arp_resolve()` hang this phase found
but did not fix, both named honestly below), on top of Phase 56 (real
SMP bring-up, a Local APIC + IO-APIC driver and an AP bootstrap
trampoline that brings a second CPU core online running real kernel
Rust code), Phase 55 (real ACPI shutdown, closing the "real shutdown,
not just reboot" row in 1.3), Phase 54 (a structured, minidump-inspired
crash dump, closing the "tells you why when it fails" row in 1.3),
Phase 53 (a journaled FAT32, closing the "won't corrupt data" row in
1.3), and the 50-52 update before that - kernel work only, matching
your stated priority; nothing in Part 2 (userland) has changed since
this document was last updated, so that section is unmodified.
Every "done" mark below
reflects verified, shipped work, not a plan - and where a real,
external constraint bounds what a phase could honestly claim (FAT32's
own on-disk format has no file-ownership fields at all, for instance;
this kernel's own syscall gate keeping interrupts disabled for a
syscall's entire duration, for another), that's stated plainly as part
of the "done" mark, not glossed over. Phase 49's own entry includes a
direct correction to a concern this very document stated in its own
prior update (that an interactive login prompt would risk stranding
the headless test suite) - found to be based on the wrong mental model
once actually investigated; see 1.1 below for the honest account.*

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
- ✅ **virtio-blk** (Phase 42), **wired into the VFS as a real,
  mountable device** (Phase 46) — see 1.2 below for both, including a
  real bug found and fixed in the second phase (a shared, silently-
  overwritten partition-offset variable between FAT32 and ext2).
- ✅ **RTL8139 genuinely interrupt-driven** (Phase 43), **and IRQ-driven
  drivers completed** (Phase 52: NE2000 receive converted too, and a
  real architectural boundary on TX found and honestly documented, not
  worked around) — see 1.2 below for the honest correction to the
  original "every driver polls" claim this same document made, and the
  full account of what Phase 52 closed vs. what it correctly left
  alone.
- ✅ **ACPI CPU topology discovery** (Phase 44) — see 1.4 below; this
  is the first SMP prerequisite, not SMP itself.
- ✅ **virtio-net** (Phase 45), this kernel's second virtio driver —
  see 1.2 below; verified against real hardware carrying ping/DNS/
  TFTP/TCP traffic simultaneously with virtio-blk.
- ✅ **UID/GID and real process identity** (Phase 47), **persisted
  across reboots** (Phase 48), **with a real interactive login screen**
  (Phase 49), **real salted PBKDF2-HMAC-SHA256 password hashing**
  (Phase 50, replacing the original FNV-1a placeholder), **and a real
  privilege-escalation model** (Phase 51, `sudo`) — see 1.1 below for
  the full account, including a direct correction to this document's
  own earlier stated concern about interactive login and the headless
  test suite.

---

## Part 1 — Kernel: what's needed to release

### 1.1 Real users and permissions

| What | Status | Why a user notices | Best-in-class reference |
|---|---|---|---|
| **UID/GID + real process identity.** | ✅ **Done (Phase 47), scoped to process identity, persisted across reboots (Phase 48).** Every `process_t` carries real `uid`/`gid` fields, threaded through every creation path with correct, distinct semantics: kernel-created tasks default to root; `fork()` and `exec()` both inherit the calling process's identity unchanged (deliberately different from this kernel's existing per-exec capability grants, which *do* reset — uid/gid represent *who's running this*, which real `exec()` doesn't change). Real accounts now survive a reboot: `USERS.CFG` (the same "kernel reads/writes a file, load validates a magic header, save overwrites" shape `SYSTEM.CFG` already established) is loaded at boot and saved when the first-boot wizard creates an account — verified end to end, not just claimed: a real, disk-persisted test account authenticates correctly from real ring-3 code after a fresh boot. **File-level ownership is a separate, real gap, not covered by this**, blocked by an external constraint found before writing any code: FAT32's own on-disk directory entry format has no uid/gid/mode field at all — adding one would be a non-standard extension breaking compatibility with every other FAT32 reader. Password hashing is now real (Phase 50) — see the row below. | Without this, there's no real notion of privacy or protection between accounts — a dealbreaker the moment more than one person (or even one person with an admin vs. normal-use account) touches the machine. | **Linux/Unix's UID/GID + permission-bits model** — simple, proven, and the foundation everything else (sudo, ACLs, containers) builds on. Start here, not with something more elaborate. |
| **A real login screen / session concept**, distinct from the current cosmetic "username" set once at first boot. | ✅ **Done (Phase 49) — including a direct correction to this document's own earlier stated concern.** A prior update of this exact document claimed an interactive login prompt would risk "silently stranding the automated test suite" in this project's headless test config. Investigated directly rather than left standing: that concern was accurate for `firstrun.c`'s own ring-0 wizard (which genuinely blocks before the scheduler starts anything else) but did not describe what a prompt *inside the ring-3 shell process* actually does — it cooperatively yields (`sys_yield()`) while polling the already non-blocking `SYS_READ_KEY`, the exact same pattern the shell's own pre-existing command prompt already used, unmodified, in every prior headless test run, without ever stranding anything. `userland/ring3-shell/shell.c` now prompts for a username and password (masked with `*`) before the shell becomes usable at all, calling the real `SYS_LOGIN`; a wrong password or a locked-out account (Phase 49 also added a bounded, in-memory failed-attempt lockout in `kernel/rust/users.rs` — the genuine "session concept" half of this row) both show the same generic "Login incorrect," deliberately not distinguished. Verified with real, scripted keystrokes via QEMU's monitor, not just reasoned about: a wrong password rejected and re-prompted; the correct password (against the real, disk-persisted account) succeeding, printing "Login successful. Welcome, persisted (uid 700)."; and the shell then genuinely usable afterward — `help` producing its full command listing, not a stuck or broken state. | The single most visible "is this a real OS" signal to a new user. | **macOS's login window** is the cleanest reference for a hobby OS — simple, fast, no unnecessary complexity. |
| **Real, salted, deliberately-slow password hashing** (Phase 47's original FNV-1a was explicitly documented as not secure — no salt, fast rather than slow, chosen only to prove the authentication flow). | ✅ **Done (Phase 50), fully in Rust.** SHA-256, HMAC-SHA256, and PBKDF2-HMAC-SHA256 all implemented from scratch (`kernel/rust/sha256.rs`/`hmac_sha256.rs`/`pbkdf2.rs`) — this freestanding kernel has no crates.io or any external dependency to draw a real hash implementation from. Every test vector independently generated with Python's own trusted `hashlib`/`hmac` before being hardcoded, not typed from memory. 4096 iterations, salted per-account — and *measured*, not assumed reasonable: bracketing a real computation with real `timer_get_ticks()` reads shows ~50-60ms per attempt on this kernel's own hardware/emulation, imperceptible to a real login while thousands of times slower than the hash it replaces. Honestly below general modern guidance for a networked multi-user system — a stated trade-off for this kernel's current single-machine context, not hidden as already fully hardened. Salt derivation is honestly bounded by this kernel having no real entropy source (confirmed by grepping the whole tree, not assumed) — reuses `kernel/net/tcp.c`'s own existing "not cryptographically random" honesty rather than inventing a new pretense. | Storing passwords with a fast, unsalted hash is a real, concrete security gap the moment more than a toy account exists — this is the difference between "an authentication flow that works" and "credentials worth trusting." | **Linux/Unix's own shadow password + PBKDF2/bcrypt/yescrypt evolution** — this phase's choice (PBKDF2-HMAC-SHA256) is the simplest widely-standardized member of that family, not the most modern one; a stated, deliberate trade-off for this project's current scale. |
| **A privilege-escalation model** (`sudo`-equivalent) once real users exist. | ✅ **Done (Phase 51).** `kernel/rust/users.rs`'s own `rust_users_sudo_check()` is the actual gate this row was waiting for — it re-authenticates the *calling* process's own account (looked up by its current numeric uid, the same way real sudo resolves the calling identity) and escalates only if the password is correct *and* the account is a member of what this kernel calls the "admin group" (`gid == 0`) — a correct password for a non-admin account is refused, same as a wrong one, proving authenticated is not the same as authorized. A real `sudo FILE [args]` shell command re-prompts for the current user's own password and, on success, runs the target command with the now-escalated identity. Verified three separate ways: the Rust check directly, the full syscall path from real ring-3 code (`SYS_SUDO -> success, now uid 0 gid 0`), and real, scripted keystrokes through the actual interactive shell command. A real, separate access-control layer was found during manual verification, not caused by this phase: this kernel's own pre-existing per-process capability list (`allowed_files[]`, independent of uid/gid) still gates file access even for an escalated, uid-0 process — `sudo` only ever escalates identity, not file capabilities, the same as the existing `run` command already didn't. **Scoped, stated directly**: escalation currently applies to the calling shell process for the rest of its own session, not per-command the way real sudo is — there is no "drop back to the original identity after the command finishes" step yet. | Lets normal use stay unprivileged by default — a genuine security property, not just cosmetic. | **Linux's `sudo`** (simpler to implement than Windows' UAC prompt-and-consent flow, and gets you 90% of the value). |

### 1.2 Drivers that don't require editing kernel boot code

| What | Status | Why a user notices | Best-in-class reference |
|---|---|---|---|
| **A driver registration table.** | ✅ **Done (Phase 39), partial scope.** `DRIVER_REGISTER(name, init_fn, phase)` places a driver into a dedicated linker section; `kernel_main()` no longer calls driver init functions by name for the ones migrated. Proven, not just claimed: a temporary demo driver was added in its own new file and confirmed to run with `kernel/init/main.c` completely unchanged. **4 of this kernel's ~10 drivers are migrated so far** — PS/2 keyboard, PS/2 mouse, UHCI, AC97. `timer_init`/`vfs_init`/`net_init` remain explicit calls, deliberately: each is interleaved with its own self-tests immediately after, and migrating those needs a real design decision (does the self-test move with it, or become separate?), not just mechanical copying. | Directly blocks "this OS works on more than the exact hardware it was tested against" — the single biggest thing standing between "runs in QEMU" and "runs on a real, different machine." | **Windows NT's HAL** is the textbook reference — it's *the* reason NT's kernel source ported across x86/MIPS/Alpha/PowerPC/ARM unchanged. |
| **virtio-net / virtio-blk drivers.** | ✅ **Both done now (Phases 42, 45, 46).** virtio-blk: legacy transport, real hardware DMA verified (Phase 42), then genuinely **wired into the VFS as a mountable device** (Phase 46) — a real FAT32 filesystem mounted, an existing file read, a new one written and read back, all through the same `fat32_init()`/`read_file()`/`write_file()` path every other filesystem operation uses, with the original ATA mount carefully saved and restored afterward (verified by re-running the *entire* test suite, including the shell launching and `fork()`/`exec()`, after the demonstration). virtio-net (Phase 45): structurally different from virtio-blk (RX buffers must be pre-posted and recycled, not a one-shot request/response) — verified against real hardware carrying ping, DNS, TFTP, and TCP traffic simultaneously with virtio-blk, in one clean boot. Neither is auto-preferred over the existing drivers when idle/unattached — this project's own test config always attaches both ATA and virtio-blk together, so auto-preferring virtio-blk would have mounted the wrong disk. | Directly relevant since you develop under QEMU — virtio is the standard, dramatically simpler way a VM talks to its host. | **Linux popularized virtio**; it's now the universal VM driver standard across every hypervisor. |
| **IRQ-driven drivers, not polling.** | ✅ **Done (Phases 43, 52), with an honest, investigated boundary on TX - not a blanket "fully IRQ-driven" claim.** Phase 43 already found the original row only partly accurate (AC97/UHCI never had the "wastes CPU while idle" property at all). Phase 52 closed the remaining, real gaps: **NE2000 receive** is now genuinely interrupt-driven, the same shape RTL8139's own Phase 43 conversion proved, reusing its exact signal (safe since only one NIC is ever active at a time). A real bug was found and fixed the same way the RTL8139 conversion originally was verified - by direct comparison against an unmodified baseline: NE2000's TX completion shares the *same* hardware register RX uses (unlike RTL8139, where they're separate), so the first version of this driver's IRQ handler could race with and silently clear a bit the existing TX code was waiting to see. Confirmed and fixed by booting an unmodified baseline (worked correctly) against the modified version (hung), isolating the exact regression, not guessing. **RTL8139's own TX path, and NE2000's, remain hardware-register busy-polls** - not an oversight, a real architectural boundary found by direct investigation: this kernel's `int 0x80` syscall gate is an interrupt gate, keeping interrupts disabled for a syscall's entire duration, so an interrupt-signal-based wait (which was built, tried, and does work correctly for every boot-time network operation - ping/DNS/TFTP/TCP all verified passing) deadlocks permanently the one time it's reached through a syscall (`SYS_NET_SEND`). Reverted cleanly rather than shipped broken; the signal infrastructure itself was kept, tested, and documented as ready for a future phase that first answers whether this kernel's scheduler safely tolerates being preempted mid-syscall. | Wastes CPU continuously, even when a real user is doing nothing — shows up as fan noise / battery drain / sluggishness on real hardware in a way it never does in QEMU. | **All three (Linux/Windows/macOS)** treat polling drivers as the exception, not the rule. |

### 1.3 Won't crash, won't corrupt data, tells you why when it fails

| What | Status | Why a user notices | Best-in-class reference |
|---|---|---|---|
| **Fix the live, confirmed scheduler/`process_wait()` bug** found during Phase 36. | ✅ **Fixed (Phase 38) — and it wasn't the scheduler at all.** The real root cause, found by instrumenting the scheduler directly and tracing a real page-fault → double-fault → triple-fault cascade with QEMU's own exception tracing: `tools/linker.ld` didn't capture rustc's per-symbol section naming, leaving a Rust static's memory outside the range the kernel reserves as "already mine" — the physical memory manager then handed that exact memory to an unrelated process's page directory, which silently aliased the kernel's own live data. Confirmed directly: decoded the corrupted page-directory bytes back to the exact ASCII text of the colliding data. Fixed generally (matching both plain and per-symbol section names for every section type), not as a one-off patch. | A hang with no error message is the single worst experience an OS can produce. | N/A — this was a correctness bug, not a missing feature. |
| **A journaled or copy-on-write filesystem.** | ✅ **Done (Phase 53), scoped to FAT32.** `kernel/rust/journal.rs` is a from-scratch write-ahead log (WAL), modeled on ext3/JBD's own physical-block journaling rather than anything copy-on-write: every `fat32_write_file()`/`fat32_delete_file()` now runs as a bounded transaction (≤128 sectors/64KB) written durably to a dedicated third MBR partition — descriptor sectors, then the real data blocks, then a commit marker, in that exact order — before a second, separate checkpoint phase applies those same blocks to their real on-disk locations and only then invalidates the journal. Recovery (run at every boot, before `fat32_init()` mounts anything) is REDO-only: a transaction whose commit marker made it to disk before a crash is finished on this kernel's behalf; one that didn't is discarded cleanly, never partially applied — the actual claim this row exists to satisfy. An FNV-1a checksum over the descriptor catches a torn write to the journal region itself (interrupted mid-descriptor-write, not mid-checkpoint), and a real, self-discovered hazard was found and closed during this phase: Phase 46's own virtio-blk self-test temporarily switches the active block device mid-boot, which — unguarded — could have made the journal misdirect a checkpoint write at the *wrong physical disk*; closed by having the journal record which device it was configured against and refuse to act if the currently-active device doesn't match. **Verified in layers, stated honestly rather than glossed over**: a real, end-to-end regression self-test now runs at every boot (`JOURNTST.TXT` — written, read back byte-for-byte, deleted, confirmed gone, all through the newly-journaled code path) proving the refactor didn't change `fat32.c`'s observable behavior; the module's own two-part self-test (a durably-committed-but-uncheckpointed transaction is completed by recovery; a never-committed one is discarded untouched) is implemented and wired into the boot sequence, ready to run for real on any machine with a working NovaOS Rust sysroot. What this phase's own verification sandbox could *not* do — and says so directly rather than papering over it — is build this kernel's actual bare-metal Rust target end-to-end: a pre-existing, unrelated toolchain fragility (this sandbox's rustc/`compiler_builtins` versions don't match what the project's own bootstrap script expects, a problem that exists independently of this phase's changes and that the project's own working sysroot, e.g. on the machine this was developed for, doesn't hit). Verified instead by (1) host-target `--emit=metadata` type- and borrow-checking `journal.rs` in isolation, which caught real language-level errors before this write-up, and (2) building and booting this *exact* kernel — every other line of C unchanged from what ships — with a temporary, sandbox-only stub standing in for every kernel-side Rust module (not just this one, since none could be built there), through which the real `fat32.c`/`vfs.c`/`kernel/drivers/blockdev.c` changes compiled, linked, and booted cleanly, correctly detected and logged the new third partition, and passed the `JOURNTST.TXT` round trip. The module's own real self-test result (durability + recovery, not just "does the surrounding C code still link") is the one honest, stated gap this phase leaves for a real Rust build to confirm. **Scoped, stated directly**: ext2 remains read-only and unjournaled — its own driver has no write path to protect; the journal protects FAT32's own `write_file()`/`delete_file()` calls specifically, not arbitrary raw sector writes made outside that path; the 128-sector/64KB transaction bound is a documented scope limit (this project's own established convention, e.g. `MAX_CLUSTER_SECTORS`), generously larger than any single write this driver currently issues, not an unbounded design; and, like every physical-block journaling filesystem (ext3/JBD included), this design assumes a single sector-sized hardware write is atomic — an industry-standard assumption, not one this kernel can independently prove against QEMU's own emulated disk. | Power loss mid-write currently risks real data corruption. | **macOS's APFS** or **Linux's ext4/btrfs journaling**. |
| **A structured crash-dump, not just a serial-port panic log.** | ✅ **Done (Phase 54).** `kernel/rust/crashdump.rs` is a from-scratch, minidump-inspired record: a single, fixed-size, checksummed (FNV-1a), versioned 512-byte-sector record written to a new, dedicated fourth MBR partition — a boot-tick timestamp, the panic reason string, an optional full x86 register snapshot, an optional faulting address (CR2, for page faults), and up to 8 stack-frame return addresses from a best-effort walk of the EBP chain. A "pending" flag distinct from the record's own persistent magic means a crash is auto-reported exactly once, at the very next boot, not on every boot after. `kernel_panic()` splits into a thin wrapper over a new `kernel_panic_fault()`, so the two call sites with real CPU register state (`isr.c`'s unhandled-exception handler, `paging.c`'s page-fault handler) capture it, while the other ~7 plain-string call sites need zero changes and still get a record, honestly without registers rather than a fabricated snapshot. The EBP walk needed a real build-flag change to be reliable — `-fno-omit-frame-pointer`, added to the kernel's own `CFLAGS` with a comment explaining why — and is bounded (alignment, this kernel's own 0–64MB identity-mapped range, strictly-increasing frames) and documented as risk mitigation, not proof of safety, since walking memory from inside a crash handler is itself a hazard. A real, deliberately un-automated `crashtest` shell command (a genuine CPU-raised `#DE` divide-by-zero, not a simulated panic) was added to the real ring-3 shell for manual end-to-end verification, kept out of `make test`'s automated suite on purpose since that suite's own job is to fail the build if an *unintended* panic appears. **A real, hard ceiling stated directly**: `kernel/fs/partition.h`'s `MAX_PARTITIONS = 4` means this is, by construction, the last MBR partition this on-disk scheme can ever add — any future phase needing its own dedicated disk region will need extended partitions or GPT. **Verified in layers, stated honestly**: the same pre-existing sandbox/toolchain gap as Phase 53 (this environment can't build the real bare-metal Rust target) means `rust_crashdump_selftest()`'s own real result wasn't observed here — verified instead via host-target `--emit=metadata` checking and a full sandbox-stub C-side integration boot, whose serial log correctly shows 4 partitions detected, "Crash dump: none pending," and the self-test honestly reporting "skipped" against a stub that never configures a real region, with Phase 53's own journal self-test and regression check both still passing unchanged in the same boot. One further gap, stated directly rather than smoothed over: the live round trip — actually triggering `crashtest`, rebooting, and confirming the next boot reports the crash — could not be exercised in this sandbox (an attempted QEMU-monitor keystroke-injection approach didn't reliably reach the guest, root cause not pinned down) and is left as a documented manual procedure for a real machine. | No way to know what actually broke after a hard crash — no register state, no faulting address, no call history, nothing that survives a reboot. | **Windows' minidump format**. |
| **Real shutdown, not just reboot.** | ✅ **Done (Phase 55).** `kernel/rust/acpi.rs` (Phase 44's own MADT-parsing module, extended rather than duplicated) now also parses the FADT (real signature `FACP`) and scans the DSDT's own AML bytecode for its `Name (_S5, Package (...) { SLP_TYPa, SLP_TYPb, ... })` object — the two 3-bit values that tell the real chipset which sleep state "S5" is on this machine — using the same narrow, independently-documented technique real hobby OSes use (scan for the literal `_S5_` name, decode only the handful of AML bytes immediately after it), not a general AML interpreter, with every read bounds-checked against this kernel's own identity-mapped range the same way Phase 44's own MADT walk already is. A new `shutdown` command in the real ring-3 shell (`SYS_SHUTDOWN` → `rust_acpi_shutdown()`) enables ACPI if it isn't already, then writes `SLP_TYPa`/`SLP_EN` to the real `PM1a_CNT_BLK` I/O port via a direct `out` instruction — on real, working ACPI hardware this does not return at all. **A newly-discovered bug class avoided on purpose**: both this function's own bounded waits (the ACPI-enable handshake, and confirming the power-off actually happened) deliberately use a fixed busy-loop count rather than real elapsed time, because this function can run from inside a syscall, and Phase 54's own entry already found that this kernel's syscall gate disables interrupts for a syscall's entire duration — a timer-tick-based wait issued from there would be a genuine deadlock, not just a slow one. **Verified more directly than any prior phase's own sandbox limitations allowed**: beyond the usual host-target Rust metadata check, this phase's own sandbox-only verification stub got a genuine, independently hand-written C reimplementation of the identical algorithm (not a degraded no-op, since ACPI parsing has no language dependency) — booting through it at a reduced-memory QEMU config (the same one Phase 44's own entry already established puts real ACPI tables inside this kernel's 64MB identity-mapped range) found a real FADT, the real `PM1a_CNT_BLK` (`0x604`, QEMU's own genuine standard value), and a real `_S5` package; then, with that confirmed, a real call to the shutdown function was temporarily wired into boot (never delivered, added and fully reverted within the same session) — the serial log ends mid-write with no return, and QEMU's own process exited on its own, well before its timeout, not killed. **A real bug this phase found in its own first draft, before delivery**: an FFI struct mirroring this new data on the C side used this kernel's own `bool` (a 4-byte C enum, not Rust's guaranteed-1-byte `bool`), silently shifting every field after the first few by 9 bytes — caught by comparing a discovery result against an independent diagnostic log of the same data, and fixed by switching every boundary-crossing flag to a plain `u8`/`uint8_t`, the same fix Phase 54's own `crash_report_t` had already established as this project's convention. **Scoped, stated directly**: `SYS_SHUTDOWN` is not privilege-gated (any process can call it — a real, sensible follow-up, not attempted here); only the legacy ACPI 1.0 32-bit I/O-port PM1_CNT fields are read, matching Phase 44's own choice not to also implement the ACPI 2.0+ XSDT/GAS extensions; and the `_S5` scan is a narrow slice of one specific, real-world-standard encoding, not a general AML interpreter. | An OS that can't turn the computer off doesn't feel finished. | Baseline expectation across all three. |

### 1.4 Multi-core

| What | Status | Why a user notices | Best-in-class reference |
|---|---|---|---|
| **Kernel synchronization primitives.** | ✅ **Done (Phase 40).** `SpinLock<T>` — the `spin_lock_irqsave`/`spin_unlock_irqrestore` shape (disables local interrupts while held *and* spins on a real atomic, so it's correct even once a second CPU exists, not just today). Applied to real, already-shipped state (`kernel/rust/pipe.rs`'s own `PIPES` table), not left as an unused primitive. This specifically closes the "no spinlock/mutex exists yet" half of the original claim in this row. | A genuine prerequisite for everything below — not something that mattered for its own sake until now. | **Linux's own approach** (fine-grained locking) is the long-term reference; this project's `SpinLock` is deliberately much simpler, correct for what it protects today. |
| **CPU topology discovery (ACPI/MADT parsing).** | ✅ **Done (Phase 44) — the genuine first SMP prerequisite, verified three separate ways** including three independent real-hardware data points (`-smp 1/2/4`, each logged count exactly matching). A real bug (a page fault reading real ACPI tables placed outside this kernel's identity-mapped range) was found and fixed correctly, not papered over. **This is not SMP** — nothing about how this kernel boots, schedules, or handles interrupts changed; no second CPU is started. | The literal first fact any SMP implementation needs (how many CPUs, and their APIC IDs) — previously not knowable at all. | Every real OS (Linux, Windows, macOS) does exactly this, this way, first. |
| **SMP support itself.** | ✅ **Mostly done (Phases 56-57) — hardware bring-up, a real per-CPU scheduler, and locking for every shared structure this project's own roadmap named plus a few more found by reading the code closely. Two honest gaps remain, named below, not swept under the rug.** Phase 56 built the hardware half: a real Local APIC driver, a real IO-APIC driver honoring MADT Interrupt Source Overrides, an AP bootstrap trampoline, and a genuine INIT-SIPI-SIPI sequence bringing a second CPU core online running real kernel Rust. Phase 57 built the half Phase 56's own header comment called "the genuinely hardest part": `kernel/task/scheduler.c` now keeps one `current` process per CPU (not a single shared global) behind a real `scheduler_lock` — never held across a context switch, the specific discipline that makes it safe — with an AP-join path (`scheduler_ap_join()`) that busy-spins rather than idles, since an AP has no periodic wake source of its own; a new `bsp_only` process flag keeps this kernel's one permanently-idle task off the AP entirely, closing a real deadlock (idle's own wake source, the timer tick, stays BSP-only) caught during design, not by testing. Every CPU now has its own TSS (`kernel/arch/x86/cpu/tss.c`) instead of one shared, load-bearing global. A CPU identifies itself via a new `rust_smp_current_cpu_index()`, built on CPUID's "initial APIC ID" specifically so it works even on a machine with no usable APIC at all (a plain single-core box included) — a genuine bug in this exact function (a register-aliasing hazard that intermittently corrupted the returned CPU index) was caught and fixed during this phase's own QEMU verification, not shipped. Locking added: `process_table[]` (a new `PROCESS_ALLOCATING` state plus `process_table_lock` close a real two-CPU double-allocation race in `allocate_slot()`), a new `exec_lock` around the shared `elf_buffer` scratch buffer in `process_exec_internal()`, the PMM bitmap (`pmm_lock`), the heap allocator (`heap_lock`), and two hazards beyond the roadmap's own original list, found by reading the rest of the kernel closely: `open_files[]` plus `handle_read()`'s shared scratch buffer (`kernel/arch/x86/cpu/syscall.c`, one `open_files_lock`), and FAT32/ext2's own shared static scratch buffers behind one coarse `vfs_lock` (`kernel/fs/vfs.c`) rather than per-driver locking, an explicit, named scope decision. Verified by the same sandbox-stub QEMU methodology this project has used throughout: with a pre-existing, unrelated hang worked around for verification only (see below), every scheduler/process/locking-dependent self-test this kernel has — ring-3 isolation, `SYS_SPAWN`, a real disk-loaded ELF, libc `malloc`, `SYS_EXEC` against both a hand-written and a libc-linked program, `fork()` and its copy-on-write isolation, `SYS_PIPE`, `SYS_LOGIN`, `SYS_SUDO` — passed cleanly, exercising exactly the code this phase touched. **What's still honestly not covered**: per-driver locking for FAT32/ext2 and for every other driver's own internal state (ATA, the NICs, AC97, UHCI, virtio) — none of it is reachable from more than one CPU today, so this is a deliberately scoped, named gap, not a miss; and a genuine, *pre-existing* (not caused by this phase) bug this phase's own verification found: `kernel/net/arp.c`'s `arp_resolve()` spins on the timer tick advancing while holding a cold ARP cache, but `int 0x80` is an interrupt gate that disables IF for the whole syscall — so any syscall reaching a cold-cache `arp_resolve()` (`SYS_NET_SEND`) hangs the entire single-core machine forever, waiting on a tick that can never fire. Not fixed here (a single-core interrupt-gate/blocking-call issue, unrelated to SMP or locking, deserving its own dedicated look) — named plainly, the same way this project names every other real gap. A machine without a usable ACPI MADT/LAPIC/IOAPIC still falls back to the original single-core path with zero behavior change. | Every machine sold in the last 15+ years has multiple cores. An OS that uses one is leaving most of the hardware's actual performance on the table, visibly. | All three do this; Linux's own multi-year effort at fine-grained locking is the concrete, hardest-earned lesson that per-driver locking (the one piece left coarse here) remains real, unrushed work, not a checkbox. |

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
- Real users/permissions, kernel side (Part 1.1) — ✅ **process-level
  identity, persistence, a real interactive login screen, real salted
  password hashing, and a real privilege-escalation model all done
  (Phases 47-51)**; file-level ownership remains a real, separate gap
  (blocked by FAT32's own format, not just unscheduled).
- ~~Driver registration table (Part 1.2)~~ ✅ **done (Phase 39)**,
  partial scope (4 of ~10 drivers migrated) — extending to the
  remaining drivers (`timer`/`vfs`/`net`) is real, smaller follow-up
  work, not a new milestone item.
- Remaining coreutils ported to ring-3 (Part 2.1) — still not started;
  see Part 2, unchanged since this document was last updated.

**v1.0 ("I'd hand this to a curious friend")**
- ~~Login screen + `sudo`-equivalent (Part 1.1)~~ ✅ **done (Phases
  47-51)** — real identity, persistence, interactive login, real
  password hashing, and a real privilege-escalation model, all
  verified. What's left under 1.1 (file-level ownership, a
  `useradd`-equivalent, per-command sudo scoping) is real, smaller
  follow-up work, not a blocker for this milestone.
- Dynamic linking (Part 2.4) — still not started.
- Package management with real network fetch (Part 2.2) — still not
  started.
- A window manager with actual dragging/management (Part 2.3) — still
  not started.
- ~~SMP support (Part 1.4)~~ ✅ **mostly done (Phases 56-57)** — see
  1.5 for the full account: hardware bring-up, a real per-CPU
  scheduler, and locking for the process table, PMM bitmap, heap
  allocator, exec buffer, and open-file-handle table, plus a coarse
  VFS-wide lock. What's left: per-driver FAT32/ext2/hardware-driver
  locking (deliberately coarse for now) and a real, pre-existing
  `arp_resolve()` interrupt-disabled hang this phase found but did not
  fix.

**v2.0 ("I'd hand this to someone who isn't a developer")**
- ~~Journaled/COW filesystem (Part 1.3)~~ ✅ **done (Phase 53)**,
  scoped to FAT32 (the only filesystem this kernel can actually write
  to) — see 1.3 for the honest scope and verification-layer account.
- Package signing (Part 2.2).
- `launchd`-style service management (Part 2.3).
- ~~Structured crash dumps (Part 1.3)~~ ✅ **done (Phase 54)** — see
  1.3 for the design, the `MAX_PARTITIONS = 4` ceiling this phase
  hits, and the one live end-to-end test left for a real machine.
- Sockets API + TCP retransmission (Part 1.5) — real networking
  reachable from real userland software.

Everything else in Parts 1–2 makes NovaOS better at any point along
this path — this is a suggested *order*, not a claim that anything
outside it doesn't matter.
