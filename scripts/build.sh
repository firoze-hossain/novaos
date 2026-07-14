#!/bin/bash
echo "🚀 Building NovaOS..."
cd "$(dirname "$0")/.."
make clean
make
if [ $? -eq 0 ]; then
    echo "✅ Build successful!"
    echo "💾 Run with: make run"
else
    echo "❌ Build failed!"
    exit 1
fi