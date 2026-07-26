# NovaOS reproducible build environment.
#
# Why this exists: the i386 cross-toolchain (gcc -m32, nasm, grub-mkrescue,
# qemu-system-i386) is easy to get on Linux, moderately annoying on
# Windows (needs WSL2 - see TESTING.md), and genuinely awkward on Apple
# Silicon Macs (needs a hand-built i686-elf cross-compiler since Apple's
# clang doesn't target bare-metal i386 ELF). This image gives every
# contributor, on every host OS, the exact same environment via Docker
# Desktop, so "works on my machine" isn't a NovaOS bug category.
#
# Usage (from the repo root):
#   docker build -t novaos-build .
#   docker run --rm -v "$PWD":/novaos -w /novaos novaos-build make
#   docker run --rm -v "$PWD":/novaos -w /novaos novaos-build make test
#
# GUI (`make run`) needs a display and is intentionally NOT run inside
# this container - see TESTING.md for the native-toolchain path when
# you want the graphical QEMU window.

FROM ubuntu:24.04

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        gcc-multilib \
        nasm \
        xorriso \
        grub-pc-bin \
        grub-common \
        mtools \
        qemu-system-x86 \
        gdb \
        make \
        git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /novaos

CMD ["make", "help"]
