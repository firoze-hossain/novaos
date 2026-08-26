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
- [ ] `store` opens the Software Center, showing Editor and Game with
      readable text (the title "NOVAOS SOFTWARE CENTER" and both
      package names should be clearly legible - this is the main
      thing to eyeball, since the font was hand-built) - clicking
      INSTALL/REMOVE with a real mouse should install or remove the
      package and update the button; **please report back if clicking
      doesn't work**, since automated headless testing of this
      specific interaction hit the same unresolved limitation Phase 7
      found for window-dragging (see PROGRESS.md's Phase 12 section)
- [ ] `lspci` lists at least a host bridge, an ISA bridge, and an IDE
      controller (all Intel/vendor `8086`) - these come from QEMU's
      base chipset emulation and should be present regardless of which
      `-device` flags were used to start the VM (Phase 13)
- [ ] `lspci` also shows an Ethernet controller, vendor `10ec` device
      `8139` (RTL8139) - `ping`/`tftp`/`pkg fetch` should all still
      work exactly as before, now running over this driver instead of
      the ISA NE2000 one (Phase 16)
- [ ] Boot output shows both `[sandbox] PASS: SYS_NET_SEND to the
      gateway succeeded` and `[sandbox] PASS: SYS_NET_SEND to
      10.0.2.100 correctly denied` (Phase 14) - the same kind of
      capability check as Phase 11's file test, now for network access
- [ ] `store` now shows each package's description on a second line,
      not just its name (Phase 15) - "A TINY TEXT EDITOR (DEMO
      PACKAGE)" should render with legible parentheses
- [ ] Boot output shows `[greeter] Hello! I was spawned by another
      process via SYS_SPAWN` plus both `[sandbox] PASS: SYS_SPAWN
      succeeded` and `[unprivileged] PASS: SYS_SPAWN correctly denied`
      (Phase 17) - the spawned process's own message is what proves it
      really ran as an independent process, not just a returned code
- [ ] `beep` plays a short tone - by default this uses a silent "none"
      audio backend (see below), so you won't actually hear anything
      unless you override `AUDIO_FLAGS` (Phase 18)
- [ ] `nslookup example.com` resolves to a real IP address, and `ping
      example.com` prints the resolved IP before pinging it (Phase
      19) - both depend on real outbound network access existing
      somewhere beneath wherever you're running this, unlike every
      other network self-test in this project
- [ ] Backspace during typing erases the previous character on screen
- [ ] `clear` clears the screen and resets the cursor
- [ ] `reboot` restarts the VM back to the GRUB menu

### About audio (Phase 18+)

`make run`/`make test` always attach an AC97 sound device, but with
QEMU's `none` audio backend by default - it accepts audio output and
silently discards it, so `beep` runs without errors but produces no
actual sound, on any machine, with no host audio hardware required.
This keeps testing portable the same way every other self-test in this
project is designed to be.

To actually **hear** `beep`, override `AUDIO_FLAGS` with a backend for
your platform, for example:
```bash
make run AUDIO_FLAGS="-audiodev pa,id=snd0 -device AC97,audiodev=snd0"      # Linux, PulseAudio
make run AUDIO_FLAGS="-audiodev coreaudio,id=snd0 -device AC97,audiodev=snd0"  # macOS
make run AUDIO_FLAGS="-audiodev dsound,id=snd0 -device AC97,audiodev=snd0"     # Windows
```

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

## Installing on real hardware or a persistent VM (Phase 20+)

`novaos.iso` is a hybrid image - the exact same file works as a
bootable CD *and* as a raw BIOS disk/USB image, verified by attaching
it as a plain QEMU hard disk (no `-cdrom`) and confirming the full
system boots identically. Run `make install-image` for the exact
commands, or directly:

```bash
# Write to a real USB drive - THIS ERASES THE DRIVE. Double-check
# the device path (lsblk/diskutil list) before running this.
sudo dd if=novaos.iso of=/dev/sdX bs=4M status=progress && sync
```

For a persistent VM (VirtualBox, VMware, QEMU, etc.), attach
`novaos.iso` as a regular hard disk - **not** a CD/DVD drive - and
attach `disk.img` as a second hard disk for persistent data (packages,
`SYSTEM.CFG`, etc.). These are still two separate images rather than
one unified disk - see PROGRESS.md's Phase 20 entry for why.

## Partitions and a second filesystem (Phase 25+)

`disk.img` is now a real partitioned disk - an MBR with partition 1
(FAT32, all the usual fixtures) and partition 2 (a real ext2
filesystem with `EXT2TEST.TXT`). `cat EXT2TEST.TXT` (or `run` reading
it, or anything else that goes through `vfs_read_file()`) works
exactly like a FAT32 file - the fallback to ext2 is transparent. Note
that ext2, unlike this project's FAT32 driver, matches filenames
**case-sensitively** (correctly reflecting how ext2 actually works) -
`cat ext2test.txt` (lowercase) will correctly report "not found," not
a bug.

See `tools/build-disk-image.sh` for exactly how the partitioned image
is built, and PROGRESS.md's Phase 25 entry for the full scope (ext2 is
read-only, root-directory-only, direct+singly-indirect blocks only).

## The real bootloader (Phase 28c)

`make test-custom-boot` boots NovaOS via a genuine, from-scratch
two-stage bootloader (`tools/custom-boot/`) instead of GRUB - the same
kernel binary, the same full self-test suite (network, USB, the
FAT32+ext2 data disk). This is completely separate from `make test`/
`make run`/`novaos.iso`/`disk.img`'s normal build - nothing about the
existing GRUB-based path is touched.

If you want to reproduce the test manually rather than via `make
test-custom-boot`, note the disk ordering requirement: `disk.img` must
be attached at IDE primary master (the fixed position this kernel's
ATA driver reads from) while the bootloader disk boots from a
different position via QEMU's `bootindex` property - see
`tools/custom-boot/test-custom-boot.sh` for the exact invocation.
Getting this backwards doesn't mean the bootloader is broken; it just
means the kernel can't find its data disk, which looks confusingly
similar to a real failure at first glance.

## USB (Phase 28b)

A UHCI controller with an emulated keyboard is attached by default
(`USB_FLAGS` in the Makefile) for every `make run`/`make test`. Watch
for `UHCI controller at PCI ...` and `USB device on port 0: address=1
vendor=0x627 ...` in the boot log - `0x627` is QEMU's own USB vendor
ID, confirming a real, correctly-decoded device descriptor. This is a
locally-emulated device with no external-connectivity dependency, so
(unlike DNS/TCP) it's a hard `make test` assertion. Keyboard *input*
(reading actual keypresses) isn't implemented yet - only enumeration.

## TCP (Phase 28a)

Watch the boot log for `TCP HTTP OK: received ... bytes from
example.com:80` - a real TCP connection, HTTP request, and response
over it, verified against a genuine external server. Like the DNS
self-test (Phase 19), this depends on real outbound network access
existing beneath wherever you're running this, so it's logged as
`[WARN]` rather than a hard failure if it can't reach the internet,
and isn't part of `make test`'s pass/fail assertion chain. No shell
command or syscall exposes this yet - `tcp_connect()`/`tcp_send()`/
`tcp_receive()`/`tcp_close()` are direct C function calls used only by
this one boot self-test so far.

## ext2 writes and fork() (Phase 26-27)

Watch the boot log for `EXT2 WRITE+READBACK OK: EXT2WROT.TXT` (Phase
26 - ext2 can now write files, not just read) and `process_fork: pid
... forked` plus `fork() + copy-on-write correctly isolated parent and
child` (Phase 27 - real `fork()`, not the `exec`-style spawn `run`
uses). Neither has a dedicated shell command - both are exercised via
the same boot self-test process (`sandbox`) that proves every other
syscall-level feature in this project, the same way `SYS_SBRK`/
`SYS_WAIT` don't have shell commands either.

## Running real programs (Phase 23+)

`run PATH [args...]` loads and runs a real ELF32 executable and waits
for it to exit, printing its real exit code. Try `run HELLO.ELF one
two` - the pre-built test executable at `tools/fixtures/HELLO.ELF`
prints a message plus whatever `argv[0]`/`argv[1]` it was given, then
exits with code 42.

That executable is written in raw assembly using NovaOS's own syscall
convention directly - see `tools/elf-fixtures/hello.asm` and
`tools/elf-fixtures/build.sh`. As of Phase 24, NovaOS also has a
minimal C library (`userland/libc/`), so you don't have to write raw
assembly for a real program anymore. Try `run HELLOC.ELF one two` -
`tools/fixtures/HELLOC.ELF` is a real C program (`userland/examples/
hello.c`) using `printf`, `argv`, and `malloc`/`strcpy`/`strcat`/
`free`, exiting with code 7.

To build your own C program against the libc:
```bash
cd userland/examples
cp hello.c myprogram.c   # edit myprogram.c, or add it to build.sh
./build.sh
```
See `userland/libc/include/` for what's available (`stdio.h`,
`stdlib.h`, `string.h`) - it's intentionally small (PROGRESS.md's
Phase 24 entry has the full list of what's not yet supported, like
`realloc` or file-stream I/O).
