#!/bin/bash
set -e

cd "$(dirname "$0")"

echo "💾 Installing Disk Analyzer..."

# Detect platform
PLATFORM=$(uname -s)
echo "🔍 Detected platform: $PLATFORM"

# Build the application
echo "🔨 Building application..."
./build.sh
RELEASE_DIR="${CARGO_TARGET_DIR:-target}/release"

# Install based on platform
case "$PLATFORM" in
    "Darwin")
        echo "🍎 Installing on macOS..."
        INSTALL_DIR="$HOME/Applications"
        mkdir -p "$INSTALL_DIR"
        ditto "$RELEASE_DIR/Discope.app" "$INSTALL_DIR/Discope.app"
        echo "Installed to $INSTALL_DIR/Discope.app"
        ;;
    "Linux")
        echo "🐧 Installing on Linux..."
        INSTALL_DIR="$HOME/.local/bin"
        mkdir -p "$INSTALL_DIR"
        cp "$RELEASE_DIR/disk-analyzer" "$INSTALL_DIR/"
        echo "✅ Installed to $INSTALL_DIR/disk-analyzer"
        echo "🎯 Make sure $INSTALL_DIR is in your PATH"
        echo "🔧 Add this to your ~/.bashrc or ~/.zshrc:"
        echo "   export PATH=\"\$HOME/.local/bin:\$PATH\""
        ;;
    "MINGW"*|"MSYS"*|"CYGWIN"*)
        echo "🪟 Installing on Windows..."
        INSTALL_DIR="$HOME/AppData/Local/Programs/disk-analyzer"
        mkdir -p "$INSTALL_DIR"
        cp "$RELEASE_DIR/disk-analyzer.exe" "$INSTALL_DIR/"
        echo "✅ Installed to $INSTALL_DIR/disk-analyzer.exe"
        echo "🎯 You can create a shortcut to this executable"
        ;;
    *)
        echo "❓ Unknown platform: $PLATFORM"
        echo "📁 Binary is available in $RELEASE_DIR"
        ;;
esac

echo ""
echo "🎉 Installation complete!"
echo "📚 For usage instructions, see README.md"


