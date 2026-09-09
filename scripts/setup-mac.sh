#!/bin/bash
echo "🍎 Setting up NovaOS for Mac..."

# Check if Homebrew is installed
if ! command -v brew &> /dev/null; then
    echo "Installing Homebrew..."
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
fi

# Install dependencies
echo "Installing dependencies..."
brew install nasm make qemu xorriso gdb coreutils mtools

# macOS has no native grub-mkrescue at all, on either architecture -
# GRUB isn't part of macOS's own boot process, and Apple doesn't ship
# or build it. x86_64-elf-grub is a real, official homebrew-core
# formula (not a community tap) with prebuilt bottles for both Apple
# Silicon and Intel Mac, confirmed directly against
# formulae.brew.sh/formula/x86_64-elf-grub - its own binary list
# explicitly includes x86_64-elf-grub-mkrescue. The "x86_64-elf"
# prefix doesn't matter for what this project actually needs from it:
# grub-mkrescue produces a BIOS-bootable, Multiboot-compliant ISO via
# GRUB's own real-mode-to-protected-mode boot process, independent of
# which architecture the GRUB binary itself was cross-compiled for -
# the same reasoning already applied to this project's own
# i686-elf-gcc/binutils cross-toolchain below.
echo "Installing GRUB (for grub-mkrescue - macOS has no native GRUB)..."
brew install x86_64-elf-grub

mkdir -p ~/.local/bin
ln -sf "$(brew --prefix x86_64-elf-grub)/bin/x86_64-elf-grub-mkrescue" ~/.local/bin/grub-mkrescue

# For Apple Silicon, install cross-compiler
if [[ $(uname -m) == "arm64" ]]; then
    echo "Apple Silicon detected - installing i686-elf cross-compiler..."
    # i686-elf-gcc/binutils live in a community tap, not homebrew-core,
    # because Apple Silicon macOS has no native way to emit bare-metal
    # i386 ELF objects (clang can't target that without a full cross
    # binutils + gcc build). This tap does that build for you.
    brew tap nativeos/i686-elf-toolchain
    brew install i686-elf-binutils i686-elf-gcc

    # Create symlinks so the Makefile's plain `gcc`/`ld` resolve to the
    # cross-compiler ahead of Apple's native (non-cross) toolchain.
    ln -sf "$(brew --prefix i686-elf-gcc)/bin/i686-elf-gcc" ~/.local/bin/gcc
    ln -sf "$(brew --prefix i686-elf-binutils)/bin/i686-elf-ld" ~/.local/bin/ld
fi

echo "Add this to your shell profile (~/.zshrc):"
echo '  export PATH="$HOME/.local/bin:$PATH"'
export PATH="$HOME/.local/bin:$PATH"

echo "✅ Mac setup complete!"
echo "Run 'make' to build NovaOS"