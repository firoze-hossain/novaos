#!/bin/bash
echo "🍎 Setting up NovaOS for Mac..."

# Check if Homebrew is installed
if ! command -v brew &> /dev/null; then
    echo "Installing Homebrew..."
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
fi

# Install dependencies
echo "Installing dependencies..."
brew install nasm make qemu xorriso gdb coreutils

# For Apple Silicon, install cross-compiler
if [[ $(uname -m) == "arm64" ]]; then
    echo "Apple Silicon detected - installing cross-compiler..."
    brew install i686-elf-gcc i686-elf-binutils

    # Create symlinks
    mkdir -p ~/.local/bin
    ln -sf /usr/local/bin/i686-elf-gcc ~/.local/bin/gcc
    ln -sf /usr/local/bin/i686-elf-ld ~/.local/bin/ld
    export PATH="$HOME/.local/bin:$PATH"
fi

echo "✅ Mac setup complete!"
echo "Run 'make' to build NovaOS"