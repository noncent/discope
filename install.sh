#!/bin/bash

echo "💾 Installing Disk Analyzer..."

# Detect platform
PLATFORM=$(uname -s)
echo "🔍 Detected platform: $PLATFORM"

# Build the application
echo "🔨 Building application..."
cargo build --release

if [ $? -ne 0 ]; then
    echo "❌ Build failed! Please check the errors above."
    exit 1
fi

# Install based on platform
case "$PLATFORM" in
    "Darwin")
        echo "🍎 Installing on macOS..."
        INSTALL_DIR="$HOME/Applications"
        mkdir -p "$INSTALL_DIR"
        cp target/release/disk-analyzer "$INSTALL_DIR/"
        echo "✅ Installed to $INSTALL_DIR/disk-analyzer"
        echo "🎯 You can now run it from Applications or use: $INSTALL_DIR/disk-analyzer"
        ;;
    "Linux")
        echo "🐧 Installing on Linux..."
        INSTALL_DIR="$HOME/.local/bin"
        mkdir -p "$INSTALL_DIR"
        cp target/release/disk-analyzer "$INSTALL_DIR/"
        echo "✅ Installed to $INSTALL_DIR/disk-analyzer"
        echo "🎯 Make sure $INSTALL_DIR is in your PATH"
        echo "🔧 Add this to your ~/.bashrc or ~/.zshrc:"
        echo "   export PATH=\"\$HOME/.local/bin:\$PATH\""
        ;;
    "MINGW"*|"MSYS"*|"CYGWIN"*)
        echo "🪟 Installing on Windows..."
        INSTALL_DIR="$HOME/AppData/Local/Programs/disk-analyzer"
        mkdir -p "$INSTALL_DIR"
        cp target/release/disk-analyzer.exe "$INSTALL_DIR/"
        echo "✅ Installed to $INSTALL_DIR/disk-analyzer.exe"
        echo "🎯 You can create a shortcut to this executable"
        ;;
    *)
        echo "❓ Unknown platform: $PLATFORM"
        echo "📁 Binary is available at: target/release/disk-analyzer"
        ;;
esac

echo ""
echo "🎉 Installation complete!"
echo "📚 For usage instructions, see README.md"


