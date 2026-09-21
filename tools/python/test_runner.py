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
# Phase 56: mirrors the Makefile's own TEST_TIMEOUT bump - see that
# variable's own comment for why (-smp 2 plus kernel/rust/apic.rs's
# own real, unconditional AP-bring-up busy-waits).
DEFAULT_TIMEOUT_SECONDS = 40

# Mirrors the Makefile's own QEMU_FLAGS/DISK_FLAGS/NET_FLAGS/
# AUDIO_FLAGS/USB_FLAGS exactly (kept here as one definition this
# script owns, rather than trying to parse them back out of the
# Makefile - if the Makefile's own flags ever change, this list needs
# a matching update, the same way any of this project's other flag
# duplication already needs to stay in sync by hand today; see
# NovaOS-Gap-Analysis-and-Kernel-Independence-Roadmap.md's own note on
# syscall.h/novasys.h for the same class of issue elsewhere in this
# project, worth keeping in mind if this script grows).
# Phase 56: -smp 2 mirrors the Makefile's own QEMU_FLAGS exactly (see
# that variable's own comment for why 2, not QEMU's default of 1 or a
# larger number) - this project's own real SMP bring-up
# (kernel/rust/apic.rs) needs more than one virtual CPU present to be
# exercised at all, and this script's own DEFAULT_TIMEOUT_SECONDS
# below already has headroom for the extra, real wall-clock cost of
# bringing up a second CPU under QEMU's software (TCG) emulation.
QEMU_BASE_FLAGS = ["-M", "pc,smm=off", "-m", "512M", "-smp", "2", "-no-reboot"]
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
    Assertion("mbr_partition_table", r"Partition table found \(MBR\): 4 partition",
              "the MBR partition table was parsed correctly (Phase 53 "
              "added a third partition - FAT32's write-ahead journal - "
              "and Phase 54 added a fourth - the crash-dump region - "
              "alongside the original FAT32 and ext2 partitions)"),
    Assertion("crashdump_none_pending", r"Crash dump: none pending",
              "the new fourth (crash-dump) partition was detected and "
              "checked at boot, and correctly reported nothing pending "
              "on a disk that was never actually crashed"),
    Assertion("crashdump_selftest",
              r"Crash dump self-test: full-record-round-trip=pass "
              r"reported-at-most-once=pass "
              r"no-registers-record-round-trip=pass",
              "the crash-dump module's own three-part self-test - a "
              "full record with a register snapshot round-trips "
              "exactly, the same slot correctly reports nothing "
              "pending on a second read, and a record with no "
              "register snapshot round-trips correctly too - ran for "
              "real against the genuinely configured fourth partition "
              "and passed"),
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
    Assertion("acpi_fadt_selftest",
              r"Kernel-side Rust ACPI FADT/_S5 parsing self-test.*"
              r"fadt-and-s5-values-correct=pass",
              "the FADT parsing and _S5 AML-package decode are both "
              "correct, verified against a synthetic FADT/DSDT with "
              "known-correct SLP_TYPa/SLP_TYPb values - the piece this "
              "kernel's real shutdown (Phase 55) needs beyond Phase "
              "44's own MADT parsing, and (like that phase's own real-"
              "hardware discovery) not itself asserted here, since "
              "whether a real FADT/_S5 are actually found depends on "
              "where this specific machine's firmware placed them "
              "relative to this kernel's own identity-mapped range - "
              "see kernel/rust/acpi.rs's own header comment"),
    Assertion("smp_aps_brought_up",
              r"SMP: [1-9]\d* application processor\(s\) brought up",
              "kernel/rust/apic.rs's own real SMP bring-up (Phase 56) "
              "found and used a Local APIC + I/O APIC pair, and at "
              "least one secondary CPU actually came online - "
              "deterministic under this project's own test config as "
              "of Phase 56 specifically because QEMU_BASE_FLAGS above "
              "now boots with `-smp 2`, unlike Phase 44's own MADT "
              "CPU-count discovery, which needed a manual `-smp N` "
              "override to verify at all"),
    Assertion("ap_running_real_code",
              r"AP online: APIC ID=0x[0-9A-F]{2} running real kernel Rust code",
              "the strongest evidence this phase's own module-level "
              "doc comment (kernel/rust/apic.rs) says to look for: not "
              "just that the hand-written real-mode trampoline ran "
              "(kernel/arch/x86/cpu/ap_trampoline.s), but that a "
              "second physical CPU core genuinely reached and is "
              "executing compiled Rust - this line is logged from "
              "inside rust_ap_main() itself, on that second core, not "
              "inferred by the boot CPU"),
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
    Assertion("sha256_selftest",
              r"Kernel-side Rust SHA-256 self-test.*empty-string=pass.*"
              r"abc=pass.*multi-block=pass",
              "this kernel's own from-scratch SHA-256 implementation "
              "matches three of the algorithm's standard, independently-"
              "known-correct test vectors exactly, including one long "
              "enough to exercise the multi-block message-schedule "
              "expansion, not just the single-block path"),
    Assertion("hmac_sha256_selftest",
              r"Kernel-side Rust HMAC-SHA256 self-test.*"
              r"rfc4231-test-case-1=pass",
              "this kernel's own HMAC-SHA256 implementation matches "
              "RFC 4231's own standard Test Case 1 exactly"),
    Assertion("pbkdf2_selftest",
              r"Kernel-side Rust PBKDF2-HMAC-SHA256 self-test.*"
              r"iterations-1=pass.*iterations-2=pass.*"
              r"iterations-4096=pass",
              "this kernel's own PBKDF2-HMAC-SHA256 implementation "
              "matches three independently-generated test vectors "
              "exactly, including this phase's own actual production "
              "iteration count (4096), not just toy cases"),
    Assertion("users_selftest_part1",
              r"Kernel-side Rust user database self-test \(1/2\): "
              r"add=pass.*persistence-round-trip=pass",
              "the UID/GID user database's add/authenticate/serialize/"
              "load round trip is correct using real, salted PBKDF2-"
              "HMAC-SHA256 password hashing (replacing Phase 47's "
              "original, explicitly-insecure FNV-1a)"),
    Assertion("users_selftest_part2",
              r"Kernel-side Rust user database self-test \(2/2\): "
              r"locked-out-after-threshold=pass.*"
              r"different-salts-for-same-password=pass",
              "an account is genuinely locked after enough failed "
              "attempts, and two accounts sharing the same password end "
              "up with different salts and different stored hashes - "
              "the actual, observable point of salting at all"),
    Assertion("users_sudo_check_selftest",
              r"sudo-wrong-password-rejected=pass sudo-admin-accepted="
              r"pass sudo-non-admin-rejected=pass",
              "the sudo re-authentication gate itself, verified directly: "
              "a wrong password is rejected, an admin-group account's own "
              "correct password is accepted, and - the actual point of "
              "the function - a genuinely correct password for a "
              "non-admin-group account is still rejected, since being "
              "authenticated is not the same as being authorized"),
    Assertion("userscfg_loaded",
              r"First-run check: USERS\.CFG loaded - accounts restored",
              "direct, standalone evidence that userscfg_load() actually "
              "read and parsed tools/fixtures/USERS.CFG from disk at "
              "boot, not just inferred from the later login test's own "
              "success"),
    Assertion("sandbox_login_passed",
              r"PASS: SYS_LOGIN/SYS_GETUID",
              "from real ring-3 code: a sandboxed process started as uid "
              "0, a wrong password was correctly rejected leaving its uid "
              "unchanged, and the correct password - checked against the "
              "real account persisted in tools/fixtures/USERS.CFG, loaded "
              "at boot via vfs_read_file - succeeded and actually changed "
              "the process's own uid to that account's real value"),
    Assertion("sandbox_sudo_passed",
              r"PASS: SYS_SUDO",
              "from real ring-3 code, through the actual syscall path (not "
              "just the direct Rust-function self-test): a non-admin "
              "account's own genuinely correct password was refused by "
              "sudo (authenticated is not authorized), a wrong password "
              "for a real admin-group account was refused, and that "
              "account's correct password succeeded, actually escalating "
              "the calling process to uid 0/gid 0 - the first check "
              "anywhere in this kernel that gates a privileged action on "
              "a process's own uid"),
    Assertion("tcp_selftest_established",
              r"Kernel-side Rust TCP self-test.*established\+backlog=pass",
              "kernel/rust/tcp.rs's real LISTEN/SYN_RECEIVED/accept state "
              "machine correctly completed a synthetic 3-way handshake"),
    Assertion("tcp_selftest_recv", r"Kernel-side Rust TCP self-test.*recv=pass",
              "kernel/rust/tcp.rs correctly delivered synthetic inbound data "
              "to rust_tcp_recv()"),
    Assertion("http_selftest_parsing",
              r"Kernel-side Rust HTTP self-test \(1/2\): "
              r"header-body-split=pass truncated-detected=pass "
              r"copy-truncation=pass",
              "kernel/rust/http.rs's own response-parsing logic correctly "
              "splits headers from body, detects a truncated (headerless) "
              "response, and never overflows the caller's own output "
              "buffer"),
    Assertion("http_selftest_ip_literal",
              r"Kernel-side Rust HTTP self-test \(2/2\): "
              r"ip-literal-parsed=pass hostname-not-misidentified=pass "
              r"digit-prefixed-hostname-ok=pass bad-octet-rejected=pass "
              r"too-few-segments-rejected=pass",
              "kernel/rust/http.rs's own IPv4-literal detection correctly "
              "parses a real dotted-quad host, never misidentifies a real "
              "hostname (including one starting with a digit) as an IP, "
              "and rejects malformed, IP-shaped input rather than "
              "accepting garbage"),
    Assertion("exec_trusted_delegation_passed",
              r"sandbox. PASS: SYS_EXEC_TRUSTED delegates can_open_any_file "
              r"correctly",
              "Phase 59's SYS_EXEC_TRUSTED syscall correctly delegates a "
              "trusted caller's own can_open_any_file capability to a "
              "child it execs this way, and plain SYS_EXEC (same caller) "
              "correctly does not - verified via a real write/read-back/"
              "delete/confirm-deleted cycle against TPROBE.ELF, a real "
              "on-disk ELF32 program, not a synthetic in-kernel call"),
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
