# NovaOS - Build & Test Environment (Windows / Linux / macOS)

This document is the single source of truth for getting a working build
of NovaOS on any of the three major desktop platforms, and for how the
project is tested automatically in CI.

## TL;DR

| Host OS | Recommended path | Also works |
|---|---|---|
| Windows 10/11 | WSL2 (Ubuntu) | Docker Desktop |
| Linux | Native | Docker |
| macOS (Intel) | Native (Homebrew) | Docker |
| macOS (Apple Silicon) | Native + cross-compiler tap | Docker |

If you just want to build and boot-test without installing anything
platform-specific, skip to **Option D: Docker** below - it's identical
on all three OSes.

---

## Option A: Windows via WSL2 (recommended)

Native Windows cannot build NovaOS directly: the kernel targets bare-metal
32-bit ELF, which needs a GCC configured for that target, NASM, and
`grub-mkrescue` - none of which MSVC/MinGW provide. WSL2 gives you a real
Ubuntu userspace with none of the performance penalty of a full VM.

1. **Install WSL2** (PowerShell, as Administrator):
   ```powershell
   wsl --install
   ```
   Reboot if prompted, then open "Ubuntu" from the Start menu and finish
   the one-time Linux user setup.

2. **Clone and set up the project inside WSL2** (not on a `/mnt/c/...`
   Windows-drive path - build there and file I/O across the 9p filesystem
   boundary is noticeably slower):
   ```bash
   cd ~
   git clone https://github.com/firoze-hossain/novaos.git
   cd novaos
   make setup
   make
   make test        # headless boot smoke test
   ```

3. **Graphical `make run`**: Windows 11's WSL2 ships WSLg, which forwards
   Linux GUI apps to your Windows desktop automatically - `make run` will
   just open a QEMU window. On Windows 10, install an X server (e.g.
   VcXsrv) and `export DISPLAY=$(ip route | awk '/default/{print $3}'):0`
   first.

## Option B: Native Linux

```bash
sudo apt update
sudo apt install -y nasm gcc-multilib xorriso grub-pc-bin grub-common \
    qemu-system-x86 gdb build-essential mtools
make
make run     # or: make test  for a headless boot check
```
(`make setup` runs the equivalent of the above automatically.)

## Option C: Native macOS

```bash
make setup   # runs scripts/setup-mac.sh
make
make run
```

Two macOS-specific notes:

- **Apple Silicon (M1/M2/M3/M4):** Apple's clang cannot emit bare-metal
  32-bit ELF objects, so `setup-mac.sh` installs a real i686-elf
  cross-compiler from the `nativeos/i686-elf-toolchain` Homebrew tap and
  symlinks it ahead of Apple's toolchain on your `PATH`.
- **Intel Macs** can use Homebrew's regular `gcc`/`nasm`/`qemu` directly.

## Option D: Docker (identical on Windows/Linux/macOS)

Best when you don't want to touch your host toolchain at all, or when
you're validating that a change builds the same way it will in CI.

```bash
docker build -t novaos-build .
docker run --rm -v "$PWD":/novaos -w /novaos novaos-build make
docker run --rm -v "$PWD":/novaos -w /novaos novaos-build make test
```

On Windows, run these from PowerShell inside Docker Desktop's WSL2 backend
(the default since Docker Desktop 4.x) - no path translation needed.

The container does **not** run `make run`'s graphical QEMU window (no
display attached); use Option A/B natively when you want to sit at the
console.

---

## Continuous Integration

`.github/workflows/ci.yml` builds the ISO and runs `make test` on every
push and pull request using `ubuntu-latest` GitHub-hosted runners, and
uploads the resulting `novaos.iso` as a build artifact. This is the same
`make test` target described below, so a green CI check means the exact
command described here passed.

---

## `make test`: the headless boot smoke test

```bash
make test
```

Boots the ISO with `-display none` and a serial log, waits up to 15
seconds (`TEST_TIMEOUT=<n> make test` to change that), and passes only if
the log contains `Interrupts enabled` and does **not** contain `PANIC` or
`FAULT`. This is what CI runs, and it's the fastest way to confirm a
change didn't break boot before you sit down for a manual `make run`
session.

This intentionally cannot verify keyboard/shell interaction end-to-end
(there's no display or input attached) - do that manually with `make run`
after `make test` passes.

## Manual interactive testing checklist

After `make run`, verify at the `nova>` prompt:

- [ ] `help` lists all commands
- [ ] `echo hello` prints `hello`
- [ ] `meminfo` shows non-zero heap AND physical-frame totals (Phase 3)
- [ ] `uptime` increases each time you re-run it
- [ ] `ls` lists `HELLO.TXT` from the test disk image (Phase 3)
- [ ] `cat HELLO.TXT` prints its contents (Phase 3)
- [ ] `ps` lists idle/shell/demo-a/demo-b processes; both demo tasks
      show TERMINATED shortly after boot (Phase 4/5)
- [ ] Both `[ring3-A] PASS` and `[ring3-B] PASS` messages appear on
      screen shortly after boot (Phase 5) - if you ever see `FAIL`
      instead, address-space isolation is broken and that's a real bug
      worth reporting, not a flaky test
- [ ] `ping 10.0.2.2` gets a reply (Phase 6) - this is QEMU's own
      user-mode networking gateway, so it works without any real
      internet access from your machine
- [ ] `gui` switches to a graphics screen showing 3 colored windows
      labeled 1/2/3 (Phase 7); dragging a window by its titlebar with
      a real mouse should move it - **please report back if dragging
      doesn't work smoothly**, since automated headless testing of
      this specific interaction hit an unresolved limitation (see
      PROGRESS.md's Phase 7 section) and could use real-world
      confirmation either way
- [ ] `ESC` inside `gui` returns cleanly to the text shell, which
      keeps responding normally to `help`/`ps`/etc. afterward
- [ ] `pkg list` shows Editor and Game as available (Phase 8)
- [ ] `pkg install Editor` succeeds, `pkg installed` then shows it,
      and `cat EDITOR.APP` prints its payload
- [ ] `pkg remove Editor` succeeds, and `pkg installed` no longer
      shows it (though `EDITOR.APP` and `EDITOR.PKG` behave
      differently here - removing only deletes the installed `.APP`
      copy, not the original `.PKG` - `pkg install Editor` should
      work again afterward)
- [ ] On a truly fresh disk image (no `SYSTEM.CFG`), booting shows the
      first-run wizard prompting for a hostname and username; on every
      later boot it should instead greet you by name and skip the
      prompts (Phase 9) - the test disk image built by `make disk.img`
      comes with a pre-seeded `SYSTEM.CFG`, so you'll need to `mdel -i
      disk.img ::SYSTEM.CFG` (via `mtools`) to see the wizard yourself
- [ ] The shell prompt shows `you@yourhost>` instead of the old
      generic `nova>`; `date`/`hostname`/`whoami` all work
- [ ] `pkg fetch Weather` downloads a package over TFTP from the
      gateway; `pkg list` then shows it as available, `pkg install
      weather` installs it, and `cat WEATHER.APP` prints its payload
      (Phase 10)
- [ ] `tftp get WEATHER.PKG` (without `pkg fetch`'s wrapping) also
      works directly, saving the raw file to disk
- [ ] `ps` shows a `sandbox` process (in addition to `demo-a`/`demo-b`)
      that has already run and shows `TERMINATED` shortly after boot
      (Phase 11) - watch the boot output for `[sandbox] PASS` (x2) and
      a `[SECURITY] ... denied SYS_OPEN('SYSTEM.CFG')` line; seeing
      `FAIL` instead of either `PASS` would mean the capability
      enforcement is broken and is worth reporting
- [ ] Backspace during typing erases the previous character on screen
- [ ] `clear` clears the screen and resets the cursor
- [ ] `reboot` restarts the VM back to the GRUB menu

### About the test disk image (Phase 3+)

`make run`/`make debug`/`make test` all depend on `disk.img`, a 64MB
FAT32 image built automatically from `tools/fixtures/` via `mtools`
(`mformat`/`mcopy`) - no loop-device mounting or root privileges
needed, so this works identically on Windows/WSL2, Linux, and macOS.
Delete `disk.img` and re-run `make run`/`make test` any time to
regenerate it from scratch; `make clean` also removes it.

If you attach your own additional `-drive` to a manual QEMU invocation,
remember `-boot order=d` - without it, the BIOS may try to boot from
the non-bootable data disk instead of the NovaOS ISO, which looks like
a silent hang (no error message, no serial output, no crash - it's just
sitting in the BIOS's own boot menu logic; see PROGRESS.md's Phase 3
notes for how this was diagnosed).

### About networking (Phase 6+)

`make run`/`make debug`/`make test` all attach a NE2000 ISA NIC using
QEMU's built-in user-mode ("SLIRP") networking - no host network
configuration, root privileges, or real internet access needed. SLIRP
always answers pings to its own gateway address (10.0.2.2) itself,
which is what the boot-time self-test and `ping 10.0.2.2` both rely on.

If you want NovaOS to reach the real internet (not just the gateway),
that depends on SLIRP's own outbound access working on your machine -
it usually does, but is out of NovaOS's control and isn't required for
anything `make test` checks.

### About networked package fetching (Phase 10+)

`make run`/`make debug`/`make test` all serve `tools/fixtures/tftproot/`
over TFTP at the gateway address (10.0.2.2) via QEMU's built-in SLIRP
TFTP server - no real network access, no separate server process to
run yourself. `pkg fetch NAME` and `tftp get FILE` both talk to that
server by default. Drop your own `.PKG` files into
`tools/fixtures/tftproot/` (or point `tftp get FILE SERVER_IP` at a
real TFTP server elsewhere) to try fetching something else.

### About graphics mode and the mouse (Phase 7+)

`gui` switches the display to VGA Mode 13h (320x200, 256 colors) using
direct hardware register programming - not QEMU's usual VBE/framebuffer
path - so it works identically whether you're running natively or
under QEMU, and returns cleanly to the text shell on ESC.

This is the one area of NovaOS that's hardest to verify headlessly:
`make test` can only confirm the PS/2 mouse *initializes* at boot, not
that dragging a window actually feels right, since that needs a real
display and a real mouse. If you try it with `make run` and notice
anything off (cursor not tracking smoothly, dragging not picking up a
window, etc.), that's genuinely useful information - see PROGRESS.md's
Phase 7 section for what was and wasn't possible to confirm through
automated testing alone.

## Debugging a boot failure

```bash
make debug
```
Launches QEMU paused (`-S`) and attaches GDB with a breakpoint on
`kernel_main`. Useful GDB commands once attached:
```
(gdb) info registers
(gdb) x/10i $eip
(gdb) continue
```

For a crash that happens *before* your own code can log anything
(e.g. a bad GDT/IDT descriptor), add `-d int,cpu_reset -D qemu.log` to
the QEMU invocation in the Makefile temporarily and inspect `qemu.log`
for the register dump at the point of the fault.
