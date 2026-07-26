# NovaOS - Project Plan (v1.0, archived)

> **Note:** This is the original project plan (July 2026), kept for
> historical reference. It has been superseded by
> [`PROJECT_PLAN.md`](../../PROJECT_PLAN.md) at the repository root,
> which reflects the combined Linux/Ubuntu/Windows-influenced vision and
> the actual Phase 2 implementation. See [`PROGRESS.md`](../../PROGRESS.md)
> for current, verified status.

## Executive Summary

NovaOS is a lightweight, educational operating system built from
scratch using C and x86 Assembly, intended as a learning platform for
OS development, a foundation for the Roze programming language
runtime, and a demonstration of low-level systems programming.

## Original Phase Plan

| Phase | Focus | Duration | Original Status |
|---|---|---|---|
| P1 | Bootloader & Kernel Foundation | 2 weeks | Complete |
| P2 | Memory Management & Interrupts | 3 weeks | In Progress |
| P3 | Filesystem & Drivers | 3 weeks | Planned |
| P4 | Process Management & Shell | 2 weeks | Planned |
| P5 | Networking & Advanced Features | 4 weeks | Planned |

## Original System Requirements

**Development Host:** x86_64 or ARM64 (Mac/Apple Silicon), macOS 10.15+
or Linux (Ubuntu 20.04+), 4GB RAM minimum, 500MB free storage.

**Target Environment:** x86 32-bit (i386), 512MB minimum memory, GRUB
ISO boot, QEMU for development.

## Original Toolchain

| Tool | Version | Purpose |
|---|---|---|
| GCC | 9.0+ | C compiler |
| NASM | 2.14+ | Assembly compiler |
| LD | 2.30+ | Linker |
| QEMU | 6.0+ | Emulator |
| GRUB | 2.02+ | Bootloader |
| Make | 4.2+ | Build system |

## Why This Was Superseded

This plan was a solid Phase 1/2 starting point but didn't yet address:

1. **A distribution-level vision.** It planned a kernel, but not the
   package manager / GUI / usability layer that makes an OS something
   a non-developer would actually use.
2. **Security as an explicit, phased roadmap** rather than an implicit
   assumption.
3. **Cross-platform build/test tooling** (Docker, CI, a headless boot
   smoke test) needed once the project has more than one contributor
   or is tested on more than one machine.
4. **Repository hygiene** - IDE project files (`.idea/`, `*.iml`) were
   committed to version control with no `.gitignore`.

All four are addressed in the current `PROJECT_PLAN.md`, `PROGRESS.md`,
`TESTING.md`, `.gitignore`, and `.github/workflows/ci.yml`.
