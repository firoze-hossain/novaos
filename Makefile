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

# Run in QEMU (interactive, graphical window)
run: $(ISO_FILE)
	$(QEMU) -cdrom $(ISO_FILE) $(QEMU_FLAGS) -vga std

# Debug with GDB
debug: $(ISO_FILE)
	$(QEMU) -cdrom $(ISO_FILE) $(QEMU_FLAGS) -s -S -vga std &
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

test: $(ISO_FILE)
	@mkdir -p $(BUILD_DIR)
	@rm -f $(TEST_LOG)
	@echo "Booting NovaOS headlessly for up to $(TEST_TIMEOUT)s..."
	@timeout $(TEST_TIMEOUT) $(QEMU) -cdrom $(ISO_FILE) $(QEMU_FLAGS) \
	    -display none -serial file:$(TEST_LOG) || true
	@echo "--- boot log ---"; cat $(TEST_LOG) || true; echo "----------------"
	@grep -q "Interrupts enabled" $(TEST_LOG) && \
	    ! grep -q "PANIC\|FAULT" $(TEST_LOG) && \
	    echo "✅ Boot test PASSED" || (echo "❌ Boot test FAILED" && exit 1)

# Clean
clean:
	rm -rf $(BUILD_DIR) $(ISO_DIR) $(ISO_FILE)

# Setup
setup:
	@./scripts/setup-$(shell ./scripts/detect-os.sh).sh

# Help
help:
	@echo "🌌 NovaOS Build Commands:"
	@echo "  make          - Build ISO"
	@echo "  make run      - Run in QEMU (graphical window)"
	@echo "  make debug    - Debug with GDB"
	@echo "  make test     - Headless boot smoke test (for CI)"
	@echo "  make clean    - Clean build files"
	@echo "  make setup    - Install dependencies"
	@echo "  make help     - Show this help"

.PHONY: all run debug test clean setup help