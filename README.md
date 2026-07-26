# NovaOS

[![Build & Boot Test](https://github.com/firoze-hossain/novaos/actions/workflows/ci.yml/badge.svg)](https://github.com/firoze-hossain/novaos/actions/workflows/ci.yml)

A from-scratch x86 operating system in C and Assembly, combining a
Linux-style modular kernel, an Ubuntu-style friendly userland/package
manager, and Windows-style usability conventions. See
[PROJECT_PLAN.md](PROJECT_PLAN.md) for the full vision and roadmap, and
[PROGRESS.md](PROGRESS.md) for exactly what's implemented today.

## Features

**Phase 1 - Bootloader & Kernel Foundation**
- Multiboot-compliant kernel, loaded via GRUB
- VGA text-mode driver with 16-color support
- Freestanding C library subset (`printf`-family, `string.h` subset)
- Bootable ISO image, runs under QEMU

**Phase 2 - Memory Management & Interrupts**
- GDT with ring 0 / ring 3 segments
- Full IDT: 32 CPU exception handlers + 16 hardware IRQs (PIC remapped,
  masked-by-default)
- PIT timer driver (100 Hz) and PS/2 keyboard driver
- `kmalloc`/`kfree` heap allocator with corruption detection
- Serial (COM1) kernel logging
- A minimal interactive shell (`help`, `echo`, `meminfo`, `uptime`, `reboot`)

**Phase 3 - Paging, Physical Memory & Filesystem**
- Physical memory manager (bitmap frame allocator, real Multiboot memory map)
- Paging enabled (identity-mapped 0-64MB), with a page-fault handler
  that decodes the faulting address instead of triple-faulting
- ATA PIO disk driver + a read-only FAT32 filesystem
- Shell `ls`/`cat` commands, backed by a real disk image (`make disk.img`)

**Phase 4 - Usermode Processes, Syscalls & Scheduling**
- A Task State Segment and real ring-3 (CPL=3) process execution
- Preemptive round-robin scheduler (context switching via a hand-written
  stack-swap primitive)
- `int 0x80` syscall interface (`SYS_WRITE`/`SYS_EXIT`/`SYS_YIELD`) -
  the only path from ring 3 into the kernel
- A demo process that actually runs at ring 3 and talks to the kernel
  only through syscalls
- Shell `ps` command

See [PROGRESS.md](PROGRESS.md) for verification details and known
limitations of the current build.

## Quick Start

Full instructions for Windows (WSL2), Linux, macOS (Intel & Apple
Silicon), and Docker are in **[TESTING.md](TESTING.md)**. Short version
for Linux/macOS:

```bash
make setup   # install dependencies for your OS
make         # build novaos.iso
make run     # boot it in QEMU
make test    # headless boot smoke test (what CI runs)
```

On Windows, install WSL2 (`wsl --install`) and run the same commands
inside the Ubuntu shell it gives you - see TESTING.md for details.

## Project Layout

```
novaos/
├── kernel/
│   ├── arch/x86/
│   │   ├── boot/       # Multiboot entry point
│   │   ├── cpu/        # GDT, TSS, IDT, ISR, IRQ, syscall, context switch
│   │   └── mm/         # PMM, paging, heap allocator
│   ├── drivers/        # vga, serial, timer, keyboard, ata
│   ├── fs/             # VFS pass-through + FAT32 (read-only)
│   ├── task/           # process table, scheduler, syscall demo task
│   ├── shell/          # minimal built-in shell
│   ├── lib/            # freestanding string/stdio subset
│   ├── include/        # public kernel headers
│   └── init/           # kernel_main and init sequencing
├── tools/              # linker script, grub.cfg, FAT32 test fixtures
├── scripts/            # per-OS setup scripts
├── .github/workflows/  # CI (build + make test on every push)
├── Dockerfile          # reproducible cross-platform build environment
├── PROJECT_PLAN.md     # vision, architecture, phased roadmap
├── PROGRESS.md         # what's actually built vs. planned
└── TESTING.md          # cross-platform build/test/debug guide
```

## Documentation

- [PROJECT_PLAN.md](PROJECT_PLAN.md) - vision, security roadmap, phases
- [PROGRESS.md](PROGRESS.md) - implementation status, verified behavior, known limitations
- [TESTING.md](TESTING.md) - build/test/debug on Windows, Linux, macOS, and Docker

## License

Add a LICENSE file appropriate to your goals for the project (MIT/GPL/etc. -
none is currently specified in this repository).
