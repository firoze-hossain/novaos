#!/usr/bin/env python3
"""
tools/python/check_prereqs.py - cross-platform build prerequisite check.

Complements, rather than replaces, scripts/setup-linux.sh/setup-mac.sh
(which *install* dependencies for one specific OS each): this checks
what's already on PATH and reports it clearly, the same way on every
platform, without installing anything itself. Useful both before a
first build (confirm setup actually worked) and after this project's
own dependencies change (confirm nothing new is missing) - the
canonical dependency list lives here, in one place, rather than
needing to be re-derived by reading three different shell scripts.

Usage:
    python3 tools/python/check_prereqs.py

Exit code is 0 if every *required* tool was found, 1 otherwise -
usable as a pre-build gate the same way tools/python/test_runner.py is
usable as a post-build one. Optional tools (gdb, rustup) are reported
but never cause a non-zero exit - this project's own build already
falls back correctly when they're absent (see tools/rust-sysroot/
build-sysroot.sh's own nightly-vs-bootstrap fallback, for instance).
"""

import shutil
import subprocess
import sys
from dataclasses import dataclass


@dataclass
class Tool:
    command: str
    required: bool
    why: str
    # A short, OS-agnostic hint - scripts/setup-linux.sh/setup-mac.sh
    # are the actual install mechanism; this is just a pointer to them
    # plus the specific package name, for anyone checking by hand.
    install_hint: str


# The canonical dependency list this project actually needs - kept
# here as the one place to update, rather than needing to stay
# separately in sync across scripts/setup-linux.sh, scripts/
# setup-mac.sh, and anyone's own mental model of "what does this
# project need." See NovaOS-Gap-Analysis-and-Kernel-Independence-
# Roadmap.md's own note on syscall.h/novasys.h for why avoiding this
# exact kind of duplication matters enough to call out explicitly.
TOOLS: list[Tool] = [
    Tool("nasm", True, "assembles the bootloader and low-level kernel code",
         "apt install nasm / brew install nasm"),
    Tool("gcc", True, "compiles this kernel's C code (32-bit, freestanding)",
         "apt install gcc build-essential / brew install i686-elf-gcc"),
    Tool("make", True, "drives the entire build - every other tool is "
         "invoked through it", "apt install make / brew install make"),
    Tool("qemu-system-x86_64", True, "boots and tests NovaOS - required "
         "for `make run`, `make test`, and `make debug`",
         "apt install qemu-system-x86 / brew install qemu"),
    Tool("xorriso", True, "builds the bootable ISO (`make`, `novaos.iso`)",
         "apt install xorriso / brew install xorriso"),
    Tool("mcopy", True, "writes files onto the FAT32 test disk image "
         "(part of the mtools package)",
         "apt install mtools / brew install mtools"),
    Tool("rustc", True, "compiles this kernel's Rust modules "
         "(kernel/rust/) and Rust userland programs",
         "apt install rustc / brew install rust, or https://rustup.rs"),
    Tool("cargo", True, "builds this project's Rust userland programs "
         "(userland/ping-rs, userland/store-rs)",
         "installed alongside rustc by rustup, or apt install cargo"),
    Tool("gdb", False, "only needed for `make debug` - optional for an "
         "ordinary build/test cycle", "apt install gdb / brew install gdb"),
    Tool("rustup", False, "only needed if this machine ever wants the "
         "real nightly + -Z build-std Rust path instead of the "
         "RUSTC_BOOTSTRAP fallback tools/rust-sysroot/build-sysroot.sh "
         "already falls back to automatically - see that script's own "
         "comments", "https://rustup.rs"),
]


def check_grub_mkrescue() -> tuple[bool, str]:
    """grub-mkrescue is checked separately from the rest of TOOLS,
    not because it's optional, but because its actual command name
    genuinely differs by platform (plain `grub-mkrescue` on Linux,
    `i686-elf-grub-mkrescue` via Homebrew on macOS - see scripts/
    setup-mac.sh's own comment on why) - a single Tool entry with one
    fixed command name would report a false negative on whichever
    platform doesn't use that exact name."""
    for name in ("grub-mkrescue", "i686-elf-grub-mkrescue"):
        if shutil.which(name) is not None:
            return True, name
    return False, ""


def tool_version(command: str) -> str:
    """Best-effort one-line version string, for tools that support
    `--version` - purely informational, never affects pass/fail."""
    for flag in ("--version", "-version"):
        try:
            result = subprocess.run(
                [command, flag], capture_output=True, text=True, timeout=5
            )
            output = (result.stdout or result.stderr).strip()
            if output:
                return output.splitlines()[0]
        except (OSError, subprocess.TimeoutExpired):
            continue
    return ""


def main() -> int:
    print("Checking NovaOS build prerequisites...\n")

    missing_required: list[Tool] = []

    for tool in TOOLS:
        found_path = shutil.which(tool.command)
        found = found_path is not None
        marker = "✅" if found else ("❌" if tool.required else "⚠️ ")
        version = tool_version(tool.command) if found else ""
        version_suffix = f" ({version})" if version else ""
        print(f"{marker} {tool.command:<20} {'found' if found else 'missing'}"
              f"{version_suffix}")
        if not found:
            print(f"   needed for: {tool.why}")
            print(f"   install: {tool.install_hint}")
            if tool.required:
                missing_required.append(tool)

    grub_found, grub_name = check_grub_mkrescue()
    marker = "✅" if grub_found else "❌"
    print(f"{marker} grub-mkrescue        "
          f"{'found (' + grub_name + ')' if grub_found else 'missing'}")
    if not grub_found:
        print("   needed for: builds the bootable ISO (`make`, `novaos.iso`)")
        print("   install: apt install grub-pc-bin (Linux) / "
              "brew install i686-elf-grub (macOS, see scripts/setup-mac.sh)")

    print()
    if missing_required or not grub_found:
        print("❌ Missing required tools - run `make setup` (this "
              "project's own OS-detecting install script), or install "
              "the specific tools listed above by hand.")
        return 1

    print("✅ All required tools found - ready to `make`.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
