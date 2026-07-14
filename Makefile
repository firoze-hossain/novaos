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
         $(CFLAGS_EXTRA)

LDFLAGS = -m elf_i386 -T tools/linker.ld $(LDFLAGS_EXTRA)
ASMFLAGS = -f elf32

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

# Run in QEMU
run: $(ISO_FILE)
	$(QEMU) -cdrom $(ISO_FILE) -m 512M -vga std

# Debug with GDB
debug: $(ISO_FILE)
	$(QEMU) -cdrom $(ISO_FILE) -m 512M -s -S -vga std &
	sleep 1
	gdb -ex "target remote localhost:1234" \
	    -ex "symbol-file $(KERNEL_BIN)" \
	    -ex "break kernel_main" \
	    -ex "continue"

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
	@echo "  make run      - Run in QEMU"
	@echo "  make debug    - Debug with GDB"
	@echo "  make clean    - Clean build files"
	@echo "  make setup    - Install dependencies"
	@echo "  make help     - Show this help"

.PHONY: all run debug clean setup help