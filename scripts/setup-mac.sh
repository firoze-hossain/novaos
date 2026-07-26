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
    mkdir -p ~/.local/bin
    ln -sf "$(brew --prefix i686-elf-gcc)/bin/i686-elf-gcc" ~/.local/bin/gcc
    ln -sf "$(brew --prefix i686-elf-binutils)/bin/i686-elf-ld" ~/.local/bin/ld
    echo "Add this to your shell profile (~/.zshrc):"
    echo '  export PATH="$HOME/.local/bin:$PATH"'
    export PATH="$HOME/.local/bin:$PATH"
fi

echo "✅ Mac setup complete!"
echo "Run 'make' to build NovaOS"