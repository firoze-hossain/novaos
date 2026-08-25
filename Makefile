# Detect OS
UNAME_S := $(shell uname -s)
ARCH := $(shell uname -m)

# Platform-specific configurations
ifeq ($(UNAME_S),Darwin)
    # Mac settings
    ifeq ($(ARCH),arm64)
        # Apple Silicon
        CC = i686-elf-gcc
        LD = i686-elf-ld
        ASM = nasm
        PREFIX :=
    else
        # Intel Mac
        CC = gcc
        LD = ld
        ASM = nasm
        PREFIX :=
    endif
    QEMU = qemu-system-i386
    GRUB_MKRESCUE = grub-mkrescue
    # Mac-specific flags
    CFLAGS_EXTRA = -Wno-builtin-declaration-mismatch
    LDFLAGS_EXTRA =
else ifeq ($(UNAME_S),Linux)
    # Linux settings
    CC = gcc
    LD = ld
    ASM = nasm
    QEMU = qemu-system-x86_64
    GRUB_MKRESCUE = grub-mkrescue
    CFLAGS_EXTRA =
    LDFLAGS_EXTRA =
else
    $(error Unsupported OS: $(UNAME_S))
endif

# Common flags
CFLAGS = -m32 -std=c99 -ffreestanding -O2 -Wall -Wextra \
         -I./kernel/include -I./kernel/drivers/vga -I./kernel/lib \
         -fno-stack-protector -fno-pie -fno-builtin -nostdlib \
         -g \
         $(CFLAGS_EXTRA)

LDFLAGS = -m elf_i386 -T tools/linker.ld $(LDFLAGS_EXTRA)
ASMFLAGS = -f elf32 -g

# QEMU flags shared by run/debug/test.
#
# `-M pc,smm=off` matters on every host, not just CI: SeaBIOS's SMM
# (legacy USB) emulation is emulated instruction-by-instruction when
# QEMU has no hardware acceleration available (no KVM on Linux, no HVF
# on Intel Mac, Hyper-V disabled on Windows) and can make even a plain
# boot take tens of seconds or appear to hang. NovaOS doesn't use SMM,
# so turning it off is free.
QEMU_FLAGS = -M pc,smm=off -m 512M -no-reboot

# Test FAT32 disk image, built with mtools (mformat/mcopy) so it works
# identically on Windows/WSL2, Linux, and macOS without needing loop-
# device mounting or root privileges. `-boot order=d` is required once
# a second drive is attached, or the BIOS may try (and silently hang,
# since it isn't bootable) to boot from the data disk instead of the
# NovaOS ISO - see TESTING.md if you ever see a boot that seems to hang
# after adding your own extra -drive.
DISK_IMG = disk.img
DISK_SIZE_MB = 64
DISK_FIXTURES_DIR = tools/fixtures
DISK_FLAGS = -boot order=d -drive file=$(DISK_IMG),format=raw,if=ide,index=0,media=disk

# Phase 6+: QEMU user-mode ("SLIRP") networking. SLIRP always answers
# pings to its own gateway address (10.0.2.2) even with no real
# outbound network access from the host/CI runner, which is what makes
# the Phase 6 ping self-test (see kernel/init/main.c) work identically
# everywhere `make test` runs. A fixed MAC keeps output reproducible.
#
# Phase 10 adds tftp=...: SLIRP runs a TFTP server on the gateway
# address serving files from this host directory - no real network
# access needed, same self-contained-test principle as the ping
# self-test above. See tools/fixtures/tftproot/.
# Phase 16: RTL8139 (PCI, I/O base auto-assigned and discovered via
# Phase 13's PCI enumeration - no fixed iobase= needed, unlike the ISA
# NE2000 device below) is now the default test NIC, preferred by
# net.c's driver-selection logic whenever present. This means every
# existing network self-test (ping, TFTP fetch, pkg fetch) now
# exercises the new driver through the entire stack, not just a
# standalone demo. The NE2000 driver remains fully present and
# functional in the tree (Phase 6) - swap the -device line below back
# to `ne2k_isa,netdev=net0,iobase=0x300,mac=...` to exercise it
# instead; net.c's fallback logic picks whichever NIC is actually
# attached.
NET_FLAGS = -netdev user,id=net0,tftp=tools/fixtures/tftproot -device rtl8139,netdev=net0,mac=52:54:00:12:34:56

# Phase 18: AC97 PCI audio, always attached with the portable "none"
# backend - a real QEMU audiodev driver that discards audio and needs
# no host audio hardware, guaranteeing `make test`/`make run` work
# identically on any machine (a CI runner with no sound card included)
# the same way every other self-test in this Makefile is designed to.
# The tradeoff: with this backend, `beep` (or the boot-time self-test)
# won't produce audible sound even on a real machine. To actually hear
# it via `make run`, override AUDIO_FLAGS with a real backend for your
# platform - e.g. `make run AUDIO_FLAGS="-audiodev pa,id=snd0 -device
# AC97,audiodev=snd0"` on Linux with PulseAudio, or `-audiodev
# coreaudio,id=snd0` on macOS. See TESTING.md.
AUDIO_FLAGS = -audiodev none,id=noaudio -device AC97,audiodev=noaudio

# Directories
KERNEL_DIR = kernel
BUILD_DIR = build
ISO_DIR = iso

# Find source files
C_SOURCES = $(shell find $(KERNEL_DIR) -name "*.c")
ASM_SOURCES = $(shell find $(KERNEL_DIR) -name "*.asm")

# Build object files
C_OBJS = $(patsubst %.c, $(BUILD_DIR)/%.o, $(C_SOURCES))
ASM_OBJS = $(patsubst %.asm, $(BUILD_DIR)/%.o, $(ASM_SOURCES))
OBJS = $(C_OBJS) $(ASM_OBJS)

# Targets
KERNEL_BIN = $(BUILD_DIR)/novaos.bin
ISO_FILE = novaos.iso

# Default target
all: $(ISO_FILE)

# Compile C files
$(BUILD_DIR)/%.o: %.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

# Compile ASM files
$(BUILD_DIR)/%.o: %.asm
	@mkdir -p $(dir $@)
	$(ASM) $(ASMFLAGS) $< -o $@

# Link kernel
$(KERNEL_BIN): $(OBJS)
	$(LD) $(LDFLAGS) -o $@ $(OBJS)

# Create ISO
$(ISO_FILE): $(KERNEL_BIN)
	@mkdir -p $(ISO_DIR)/boot/grub
	cp $(KERNEL_BIN) $(ISO_DIR)/boot/novaos.bin
	cp tools/grub.cfg $(ISO_DIR)/boot/grub/
	$(GRUB_MKRESCUE) -o $(ISO_FILE) $(ISO_DIR)
	@echo "✅ NovaOS ISO created: $(ISO_FILE)"

# Create the test disk image (Phase 3+; genuinely partitioned as of
# Phase 25 - see tools/build-disk-image.sh for the full build process
# and PROGRESS.md for why). Requires `parted`, `mtools` (mformat/
# mcopy), and `e2fsprogs` (mkfs.ext2, debugfs) - installed by `make
# setup` on every supported OS.
$(DISK_IMG): $(DISK_FIXTURES_DIR)/HELLO.TXT $(DISK_FIXTURES_DIR)/EDITOR.PKG $(DISK_FIXTURES_DIR)/GAME.PKG $(DISK_FIXTURES_DIR)/SYSTEM.CFG $(DISK_FIXTURES_DIR)/HELLO.ELF $(DISK_FIXTURES_DIR)/HELLOC.ELF $(DISK_FIXTURES_DIR)/ext2root/EXT2TEST.TXT tools/build-disk-image.sh
	./tools/build-disk-image.sh $(DISK_IMG)
	@echo "✅ Partitioned test disk image created: $(DISK_IMG)"

# Run in QEMU (interactive, graphical window)
run: $(ISO_FILE) $(DISK_IMG)
	$(QEMU) -cdrom $(ISO_FILE) $(DISK_FLAGS) $(NET_FLAGS) $(AUDIO_FLAGS) $(QEMU_FLAGS) -vga std

# Debug with GDB
debug: $(ISO_FILE) $(DISK_IMG)
	$(QEMU) -cdrom $(ISO_FILE) $(DISK_FLAGS) $(NET_FLAGS) $(AUDIO_FLAGS) $(QEMU_FLAGS) -s -S -vga std &
	sleep 1
	gdb -ex "target remote localhost:1234" \
	    -ex "symbol-file $(KERNEL_BIN)" \
	    -ex "break kernel_main" \
	    -ex "continue"

# Headless boot smoke test: boots NovaOS with no display, captures the
# serial log, and fails (non-zero exit) if the expected subsystem
# init markers are missing. This is what scripts/test.sh and CI use,
# and it works identically on Linux, macOS, and Windows/WSL2.
TEST_TIMEOUT ?= 15
TEST_LOG = build/test-serial.log

test: $(ISO_FILE) $(DISK_IMG)
	@mkdir -p $(BUILD_DIR)
	@rm -f $(TEST_LOG)
	@echo "Booting NovaOS headlessly for up to $(TEST_TIMEOUT)s..."
	@timeout $(TEST_TIMEOUT) $(QEMU) -cdrom $(ISO_FILE) $(DISK_FLAGS) $(NET_FLAGS) $(AUDIO_FLAGS) $(QEMU_FLAGS) \
	    -display none -serial file:$(TEST_LOG) || true
	@echo "--- boot log ---"; cat $(TEST_LOG) || true; echo "----------------"
	@grep -q "Interrupts enabled" $(TEST_LOG) && \
	    grep -q "FAT32 mounted" $(TEST_LOG) && \
	    grep -q "FILE READ OK: HELLO.TXT" $(TEST_LOG) && \
	    grep -q "Partition table found (MBR): 2 partition" $(TEST_LOG) && \
	    grep -q "ext2 mounted: block_size=4096" $(TEST_LOG) && \
	    grep -q "EXT2 FILE READ OK: EXT2TEST.TXT" $(TEST_LOG) && \
	    grep -q "ring3-A. PASS" $(TEST_LOG) && \
	    grep -q "ring3-B. PASS" $(TEST_LOG) && \
	    grep -q "PING OK" $(TEST_LOG) && \
	    grep -q "TFTP FETCH OK" $(TEST_LOG) && \
	    grep -q "PKG INSTALL OK" $(TEST_LOG) && \
	    grep -q "PKG REMOVE OK" $(TEST_LOG) && \
	    grep -q "First-run check: returning user" $(TEST_LOG) && \
	    grep -q "sandbox. PASS: HELLO.TXT opened" $(TEST_LOG) && \
	    grep -q "sandbox. PASS: SYS_OPEN" $(TEST_LOG) && \
	    grep -q "PCI ENUMERATION OK" $(TEST_LOG) && \
	    grep -q "vendor=0x8086 device=0x1237" $(TEST_LOG) && \
	    grep -q "Network up (RTL8139)" $(TEST_LOG) && \
	    grep -q "sandbox. PASS: SYS_SPAWN succeeded" $(TEST_LOG) && \
	    grep -q "unprivileged. PASS: SYS_SPAWN correctly denied" $(TEST_LOG) && \
	    grep -q "greeter. Hello" $(TEST_LOG) && \
	    grep -q "Hello from a real ELF executable" $(TEST_LOG) && \
	    grep -q "HELLO.ELF. .pid .*. exited with code 42" $(TEST_LOG) && \
	    grep -q "malloc.d string: it works!" $(TEST_LOG) && \
	    grep -q "HELLOC.ELF. .pid .*. exited with code 7" $(TEST_LOG) && \
	    grep -q "SYS_EXEC loaded and ran a real C program" $(TEST_LOG) && \
	    grep -q "sandbox. PASS: SYS_EXEC loaded and ran" $(TEST_LOG) && \
	    grep -q "AC97 audio at PCI" $(TEST_LOG) && \
	    grep -q "AC97 beep: playing" $(TEST_LOG) && \
	    grep -q "sandbox. PASS: SYS_NET_SEND to the gateway" $(TEST_LOG) && \
	    grep -q "SECURITY. pid .* denied SYS_NET_SEND" $(TEST_LOG) && \
	    grep -q "SECURITY. pid .* denied SYS_OPEN" $(TEST_LOG) && \
	    ! grep -q "PANIC\|FAULT\|FAIL" $(TEST_LOG) && \
	    echo "✅ Boot test PASSED" || (echo "❌ Boot test FAILED" && exit 1)

# Clean
clean:
	rm -rf $(BUILD_DIR) $(ISO_DIR) $(ISO_FILE) $(DISK_IMG)

# Setup
setup:
	@./scripts/setup-$(shell ./scripts/detect-os.sh).sh

# Help
help:
	@echo "🌌 NovaOS Build Commands:"
	@echo "  make          - Build ISO"
	@echo "  make disk.img - Build the FAT32 test disk image"
	@echo "  make run      - Run in QEMU (graphical window)"
	@echo "  make debug    - Debug with GDB"
	@echo "  make test     - Headless boot smoke test (for CI)"
	@echo "  make clean    - Clean build files"
	@echo "  make setup    - Install dependencies"
	@echo "  make install-image - Build a bootable image + print USB/VM install instructions"
	@echo "  make help     - Show this help"

# Phase 20: novaos.iso is a hybrid image (xorriso/grub-mkrescue's
# default output) - the same file boots both as an El Torito CD *and*
# as a raw BIOS hard disk/USB image via its embedded MBR. This is
# NovaOS's real installer: no separate bootloader-writing step is
# needed, because grub-mkrescue already produces one. Verified by
# attaching novaos.iso directly as a QEMU hard disk (not -cdrom) and
# confirming the full system - FAT32, networking, everything - boots
# identically (see PROGRESS.md's Phase 20 entry).
install-image: $(ISO_FILE) $(DISK_IMG)
	@echo ""
	@echo "✅ $(ISO_FILE) is a hybrid image: it boots as a CD *and* as a"
	@echo "   raw BIOS disk/USB image from the same file - this is"
	@echo "   NovaOS's installer. To put it on a real USB drive (this ERASES"
	@echo "   the drive - double check the device path first):"
	@echo ""
	@echo "     sudo dd if=$(ISO_FILE) of=/dev/sdX bs=4M status=progress && sync"
	@echo ""
	@echo "   To run persistently in a VM (VirtualBox/VMware/QEMU/etc.),"
	@echo "   attach $(ISO_FILE) as a regular hard disk (NOT a CD/DVD drive)."
	@echo "   Attach $(DISK_IMG) as a second hard disk for persistent data"
	@echo "   (packages, SYSTEM.CFG, etc.) - see TESTING.md for why these"
	@echo "   are still two separate images rather than one, for now."

.PHONY: all run debug test clean setup help install-image