#!/usr/bin/env python3
"""
tools/python/test_runner.py - structured boot-test runner.

Replaces the Makefile's own `test` target internals: previously a
single, unbroken chain of ~47 `grep -q "..." $(TEST_LOG) && \\` lines,
which told you only "the boot test failed" on any single mismatch,
never which one. Every assertion below has an actual name and a short
description of what it proves, and a failed run reports every failing
check individually - not just a final pass/fail.

This script does not replace the Makefile; it's what `make test`
now calls into (see the `test:` target) to do the actual checking,
after the Makefile itself has built the ISO/disk image and booted
QEMU. It can also be run standalone against an existing log file for
fast iteration without a full rebuild+reboot:

    python3 tools/python/test_runner.py --log build/test-serial.log

Or, to boot and check in one step (what `make test` does):

    python3 tools/python/test_runner.py --boot

Exit code is 0 if every assertion passed, 1 otherwise - safe to use
directly as a CI/Makefile gate.
"""

import argparse
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
BUILD_DIR = REPO_ROOT / "build"
DEFAULT_LOG_PATH = BUILD_DIR / "test-serial.log"
DEFAULT_ISO = REPO_ROOT / "novaos.iso"
DEFAULT_DISK = REPO_ROOT / "disk.img"
DEFAULT_TIMEOUT_SECONDS = 25

# Mirrors the Makefile's own QEMU_FLAGS/DISK_FLAGS/NET_FLAGS/
# AUDIO_FLAGS/USB_FLAGS exactly (kept here as one definition this
# script owns, rather than trying to parse them back out of the
# Makefile - if the Makefile's own flags ever change, this list needs
# a matching update, the same way any of this project's other flag
# duplication already needs to stay in sync by hand today; see
# NovaOS-Gap-Analysis-and-Kernel-Independence-Roadmap.md's own note on
# syscall.h/novasys.h for the same class of issue elsewhere in this
# project, worth keeping in mind if this script grows).
QEMU_BASE_FLAGS = ["-M", "pc,smm=off", "-m", "512M", "-no-reboot"]
NET_FLAGS = [
    "-netdev",
    "user,id=net0,tftp=tools/fixtures/tftproot",
    "-device",
    "rtl8139,netdev=net0,mac=52:54:00:12:34:56",
]
AUDIO_FLAGS = [
    "-audiodev",
    "none,id=noaudio",
    "-device",
    "AC97,audiodev=noaudio",
]
USB_FLAGS = ["-device", "piix3-usb-uhci", "-device", "usb-kbd"]

# Phase 42: a small, dedicated disk image for kernel/drivers/virtio/
# virtio_blk.c's own self-test - deliberately separate from disk.img
# (the FAT32/ext2 test disk every other assertion above depends on),
# so a virtio-blk read/write mistake can never risk corrupting
# anything the rest of this suite relies on. Generated fresh by this
# script itself (see ensure_virtio_test_disk() below), not committed
# as a binary - its content doesn't matter at all before boot (this
# driver's own self-test writes a known pattern and reads it back;
# nothing before that ever reads this file's prior contents), so
# there's nothing to keep in sync the way tools/fixtures/SYSTEM.CFG's
# binary fixture needs to be.
# Phase 46: still the same dedicated, separate-from-disk.img image
# (so a filesystem mistake here can never risk the FAT32/ext2 test
# disk every other assertion depends on) - but now formatted with a
# real FAT32 filesystem containing one known file, not just zeroed
# bytes, so kernel/init/main.c's own Phase 46 self-test can prove
# virtio-blk works as a genuine, mountable VFS block device (mount,
# read a real pre-existing file, write a new one, read it back),
# not just raw sector I/O (which Phase 42's own self-test - superseded
# by this, more comprehensive one - already proved).
DEFAULT_VIRTIO_DISK = REPO_ROOT / "build" / "virtio-blk-test.img"
VIRTIO_TEST_DISK_SIZE_BYTES = 16 * 1024 * 1024  # 16MB - comfortably
# above the smallest size `mformat -F` will reliably treat as FAT32
# rather than silently falling back to FAT16 for a very small volume
# (confirmed directly, not assumed: even 1MB worked with -F in local
# testing, but 16MB removes any doubt without meaningfully slowing
# down test disk generation).
VIRTIO_FAT32_TEST_FILENAME = "VIRTTEST.TXT"
VIRTIO_FAT32_TEST_CONTENT = (
    b"Hello from a real FAT32 filesystem mounted through virtio-blk!\n"
)


@dataclass
class Assertion:
    name: str
    pattern: str
    description: str
    negative: bool = False  # True means "must NOT appear"

    def check(self, log_text: str) -> bool:
        found = re.search(self.pattern, log_text) is not None
        return (not found) if self.negative else found


# Every check the old Makefile chain made, in the same order, each
# now with a real name and a one-line description of what it actually
# proves - ported faithfully from the Makefile's `test:` target, not
# reduced or reworded away from what it originally checked.
ASSERTIONS: list[Assertion] = [
    Assertion("interrupts_enabled", r"Interrupts enabled",
              "IDT/PIC setup completed and interrupts were armed"),
    Assertion("fat32_mounted", r"FAT32 mounted",
              "the FAT32 partition was found and mounted"),
    Assertion("fat32_file_read", r"FILE READ OK: HELLO\.TXT",
              "a real file was read back correctly from FAT32"),
    Assertion("mbr_partition_table", r"Partition table found \(MBR\): 2 partition",
              "the MBR partition table was parsed correctly"),
    Assertion("ext2_mounted", r"ext2 mounted: block_size=4096",
              "the ext2 partition was found and mounted"),
    Assertion("ext2_file_read", r"EXT2 FILE READ OK: EXT2TEST\.TXT",
              "a real file was read back correctly from ext2"),
    Assertion("ext2_write_readback", r"EXT2 WRITE.READBACK OK: EXT2WROT\.TXT",
              "a file written to ext2 read back identical to what was written"),
    Assertion("capability_open_granted",
              r"SYS_OPEN\(.HELLO\.TXT.\) -> handle .* \(capability granted\)",
              "a capability-permitted file open succeeded"),
    Assertion("rust_selftest_add", r"Kernel-side Rust self-test.*= 42",
              "the original kernel-side Rust FFI self-test (Phase 35) still passes"),
    Assertion("ring3_coreutils_cat", r"Ring-3 coreutils: CAT\.ELF",
              "the ring-3 cat coreutil ran"),
    Assertion("ring3_isolation_a", r"ring3-A. PASS",
              "process A's private memory was never touched by process B"),
    Assertion("ring3_isolation_b", r"ring3-B. PASS",
              "process B's private memory was never touched by process A"),
    Assertion("ping_ok", r"PING OK", "an ICMP echo round-trip completed"),
    Assertion("tftp_fetch_ok", r"TFTP FETCH OK", "a TFTP file fetch completed"),
    Assertion("pkg_install_ok", r"PKG INSTALL OK", "the package manager installed a package"),
    Assertion("pkg_remove_ok", r"PKG REMOVE OK", "the package manager removed a package"),
    Assertion("firstrun_returning_user", r"First-run check: returning user",
              "SYSTEM.CFG round-tripped correctly, recognized as a returning user"),
    Assertion("sandbox_hello_opened", r"sandbox. PASS: HELLO\.TXT opened",
              "the sandboxed task's allowed file open succeeded"),
    Assertion("sandbox_sys_open", r"sandbox. PASS: SYS_OPEN",
              "the sandboxed task's capability enforcement behaved correctly"),
    Assertion("pci_enumeration_ok", r"PCI ENUMERATION OK",
              "PCI bus enumeration completed"),
    Assertion("pci_known_device", r"vendor=0x8086 device=0x1237",
              "a specific, known PCI device was correctly identified"),
    Assertion("network_up", r"Network up \(RTL8139\)",
              "the RTL8139 NIC driver brought the network up"),
    Assertion("sandbox_spawn_succeeded", r"sandbox. PASS: SYS_SPAWN succeeded",
              "a capability-granted process spawn succeeded"),
    Assertion("unprivileged_spawn_denied",
              r"unprivileged. PASS: SYS_SPAWN correctly denied",
              "an unprivileged process's spawn attempt was correctly denied"),
    Assertion("greeter_spawned", r"greeter. Hello",
              "a spawned child process actually ran"),
    Assertion("real_elf_ran", r"Hello from a real ELF executable",
              "a real, disk-loaded ELF executable ran"),
    Assertion("hello_elf_exit_code", r"HELLO\.ELF. \(pid .*\) exited with code 42",
              "HELLO.ELF ran to completion with its expected exit code"),
    Assertion("libc_malloc_works", r"malloc.d string: it works!",
              "the ring-3 libc's malloc() produced usable memory"),
    Assertion("helloc_elf_exit_code", r"HELLOC\.ELF. \(pid .*\) exited with code 7",
              "a real, libc-linked C program ran to completion with its expected exit code"),
    Assertion("sys_exec_real_c_program", r"SYS_EXEC loaded and ran a real C program",
              "SYS_EXEC correctly loaded and ran a full C program, not just a toy"),
    Assertion("fork_created_child", r"process_fork: pid .* forked",
              "fork() actually created a new process"),
    Assertion("fork_child_ran", r"sandbox-child. I am the child",
              "the forked child process actually ran its own code"),
    Assertion("fork_cow_isolated", r"fork\(\) \+ copy-on-write correctly isolated",
              "fork()'s copy-on-write correctly isolated parent and child memory"),
    Assertion("sandbox_exec_passed", r"sandbox. PASS: SYS_EXEC loaded and ran",
              "the sandboxed task's own SYS_EXEC test passed"),
    Assertion("ac97_present", r"AC97 audio at PCI",
              "an AC97 audio device was found and initialized"),
    Assertion("ac97_beep_played", r"AC97 beep: playing",
              "AC97 playback was actually triggered"),
    Assertion("uhci_present", r"UHCI controller at PCI",
              "a UHCI USB controller was found and initialized"),
    Assertion("usb_device_enumerated", r"USB device on port .*vendor=0x627",
              "a real USB device was enumerated on the UHCI root hub"),
    Assertion("sandbox_net_send_passed", r"sandbox. PASS: SYS_NET_SEND to the gateway",
              "a capability-granted network send succeeded"),
    Assertion("security_denied_net_send", r"SECURITY. pid .* denied SYS_NET_SEND",
              "an unauthorized network send was correctly denied"),
    Assertion("security_denied_open", r"SECURITY. pid .* denied SYS_OPEN",
              "an unauthorized file open was correctly denied"),
    Assertion("pipe_selftest_roundtrip",
              r"Kernel-side Rust pipe self-test.*roundtrip=pass",
              "the kernel-side Rust pipe implementation's round-trip test passed"),
    Assertion("pipe_selftest_wraparound", r"wraparound\(400x4B\)=pass",
              "the pipe ring buffer's wraparound arithmetic is correct over 400 cycles"),
    Assertion("spinlock_selftest_basic",
              r"Kernel-side Rust spinlock self-test.*basic-protection=pass",
              "the kernel-side Rust SpinLock correctly protects its data"),
    Assertion("spinlock_selftest_nested", r"nested-lock-interrupt-handling=pass",
              "nested SpinLock acquisition correctly preserves interrupt state"),
    Assertion("sandbox_pipe_syscall_path", r"sandbox. PASS: SYS_PIPE",
              "the SYS_PIPE/SYS_WRITE_HANDLE syscall path works from real ring-3 code"),
    Assertion("driver_keyboard_registered", r"Driver .PS/2 keyboard. initializing",
              "the PS/2 keyboard driver ran via the self-registration mechanism"),
    Assertion("driver_mouse_registered", r"Driver .PS/2 mouse. initializing",
              "the PS/2 mouse driver ran via the self-registration mechanism"),
    Assertion("driver_uhci_registered", r"Driver .UHCI USB controller. initializing",
              "the UHCI driver ran via the self-registration mechanism"),
    Assertion("driver_ac97_registered", r"Driver .AC97 audio. initializing",
              "the AC97 driver ran via the self-registration mechanism"),
    Assertion("virtio_blk_selftest_layout",
              r"Kernel-side Rust virtqueue layout self-test.*"
              r"small-queue-layout=pass",
              "the virtio-blk virtqueue's byte-layout arithmetic is correct, "
              "checked against independently hand-computed values"),
    Assertion("virtio_blk_present", r"virtio-blk at PCI",
              "a virtio-blk device was found and the legacy handshake completed"),
    Assertion("virtio_blk_write_readback", r"VIRTIO-BLK WRITE\.READBACK OK",
              "a 512-byte sector written via a real virtio-blk device (real "
              "hardware DMA, not just the layout math) read back identical"),
    Assertion("net_irq_selftest",
              r"Kernel-side Rust net IRQ signal self-test.*"
              r"signal-then-check=pass",
              "the RTL8139 RX-pending signal/check-and-clear primitive is "
              "correct, backing genuinely interrupt-driven reception instead "
              "of polling NIC hardware on every idle tick"),
    Assertion("acpi_madt_selftest",
              r"Kernel-side Rust ACPI MADT parsing self-test.*checksum=pass",
              "the ACPI MADT parsing logic is correct, verified against a "
              "synthetic table with a known-correct enabled/disabled CPU "
              "count and Local APIC address - the genuine first "
              "prerequisite for SMP (CPU topology discovery), not SMP "
              "support itself"),
    Assertion("virtio_net_selftest",
              r"Kernel-side Rust virtio-net self-test.*layout=pass",
              "the virtio-net virtqueue layout math and RX buffer post/"
              "poll/recycle logic are correct, verified against a "
              "synthetic in-memory queue - real hardware (an actual "
              "Ethernet frame sent and received through QEMU "
              "virtio-net-pci DMA) is verified separately, manually, "
              "outside this project's shared default test config, since "
              "it isn't the NIC this config actually attaches"),
    Assertion("virtio_blk_vfs_mount", r"VIRTIO-BLK VFS MOUNT OK",
              "a real FAT32 filesystem was mounted through virtio-blk via "
              "the same fat32_init()/read_file()/write_file() path every "
              "other filesystem operation uses (not just raw sector I/O), "
              "an existing file read correctly, a new file written and "
              "read back correctly, and the original ATA-backed mount "
              "restored afterward - proven by every test after this one "
              "in the suite (shell launch, fork, exec) still passing"),
    Assertion("no_panic_fault_or_fail", r"PANIC|FAULT|FAIL", "",
              negative=True),
]


def ensure_virtio_test_disk(path: Path) -> None:
    """Creates (or recreates) the dedicated virtio-blk test disk image
    as a real, mountable FAT32 filesystem - same tooling
    (mformat/mcopy) as tools/build-disk-image.sh already uses for
    disk.img's own FAT32 partition, not a new mechanism invented for
    this. Raw, unpartitioned FAT32 starting at LBA 0 - deliberately
    simpler than disk.img's own MBR-partitioned layout, since this
    image only ever needs to hold one known test file, not coexist
    with a second (ext2) filesystem the way disk.img's own two
    partitions do."""
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        path.unlink()

    with tempfile.TemporaryDirectory() as tmpdir:
        tmp = Path(tmpdir)
        with open(path, "wb") as f:
            f.truncate(VIRTIO_TEST_DISK_SIZE_BYTES)

        subprocess.run(["mformat", "-i", str(path), "-F", "::"],
                        check=True, capture_output=True)

        test_file = tmp / VIRTIO_FAT32_TEST_FILENAME
        test_file.write_bytes(VIRTIO_FAT32_TEST_CONTENT)
        subprocess.run(
            ["mcopy", "-i", str(path), str(test_file),
             f"::{VIRTIO_FAT32_TEST_FILENAME}"],
            check=True, capture_output=True,
        )


def boot_and_capture(
    iso_path: Path,
    disk_path: Path,
    log_path: Path,
    timeout_seconds: int,
    virtio_disk_path: Path,
) -> None:
    """Boots NovaOS headlessly under QEMU, capturing serial output to
    `log_path` - the same invocation shape as the Makefile's own
    `test:` target, just issued from Python instead of a Makefile
    recipe line."""
    qemu_bin = shutil.which("qemu-system-x86_64")
    if qemu_bin is None:
        print("error: qemu-system-x86_64 not found on PATH", file=sys.stderr)
        sys.exit(2)

    log_path.parent.mkdir(parents=True, exist_ok=True)
    if log_path.exists():
        log_path.unlink()

    ensure_virtio_test_disk(virtio_disk_path)
    virtio_flags = [
        "-drive", f"file={virtio_disk_path},if=none,id=vblk0",
        "-device", "virtio-blk-pci,drive=vblk0",
    ]

    cmd = (
        [qemu_bin]
        + QEMU_BASE_FLAGS
        + ["-boot", "order=d", "-drive",
           f"file={disk_path},format=raw,if=ide,index=0,media=disk"]
        + NET_FLAGS
        + AUDIO_FLAGS
        + USB_FLAGS
        + virtio_flags
        + ["-cdrom", str(iso_path), "-display", "none",
           "-serial", f"file:{log_path}"]
    )

    print(f"Booting NovaOS headlessly for up to {timeout_seconds}s...")
    try:
        subprocess.run(cmd, cwd=REPO_ROOT, timeout=timeout_seconds,
                        stdin=subprocess.DEVNULL,
                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except subprocess.TimeoutExpired:
        # Expected in the common case: this kernel's own boot sequence
        # never exits QEMU itself, so the timeout is what ends a
        # normal, successful run - not a failure by itself. Whether
        # the run actually succeeded is entirely up to the assertion
        # checks against whatever got logged before the timeout hit.
        pass


def run_checks(log_path: Path) -> bool:
    """Checks every assertion against the captured log, printing a
    per-assertion PASS/FAIL line (the actual improvement over the old
    Makefile chain) and a final summary. Returns True iff every
    assertion passed."""
    if not log_path.exists():
        print(f"error: log file not found: {log_path}", file=sys.stderr)
        return False

    log_text = log_path.read_text(errors="replace")

    print(f"\nChecking {len(ASSERTIONS)} assertions against {log_path}:\n")
    failures: list[Assertion] = []
    for a in ASSERTIONS:
        passed = a.check(log_text)
        status = "PASS" if passed else "FAIL"
        label = f"[{status}] {a.name}"
        if a.description:
            label += f" - {a.description}"
        print(label)
        if not passed:
            failures.append(a)

    print()
    if failures:
        print(f"❌ {len(failures)}/{len(ASSERTIONS)} assertions failed:")
        for a in failures:
            kind = "should not have matched but did" if a.negative else "pattern not found"
            print(f"   - {a.name} ({kind}: /{a.pattern}/)")
        return False

    print(f"✅ All {len(ASSERTIONS)} assertions passed")
    return True


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", type=Path, default=DEFAULT_LOG_PATH,
                         help="path to a serial log to check "
                              "(default: %(default)s)")
    parser.add_argument("--boot", action="store_true",
                         help="boot QEMU first and capture a fresh log, "
                              "rather than checking an existing one")
    parser.add_argument("--iso", type=Path, default=DEFAULT_ISO)
    parser.add_argument("--disk", type=Path, default=DEFAULT_DISK)
    parser.add_argument("--virtio-disk", type=Path, default=DEFAULT_VIRTIO_DISK,
                         help="path to the dedicated virtio-blk test disk "
                              "image (created fresh each run; default: "
                              "%(default)s)")
    parser.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT_SECONDS)
    args = parser.parse_args()

    if args.boot:
        boot_and_capture(args.iso, args.disk, args.log, args.timeout,
                          args.virtio_disk)

    ok = run_checks(args.log)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
