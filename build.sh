#!/bin/bash

echo "🚀 Building Disk Analyzer..."

# Build the release version
cargo build --release

if [ $? -eq 0 ]; then
    echo "✅ Build successful!"
    echo "📁 Binary location: target/release/disk-analyzer"
    echo ""
    echo "🎯 To run the application:"
    echo "   ./target/release/disk-analyzer"
    echo ""
    echo "🔧 Or use cargo:"
    echo "   cargo run --release"
else
    echo "❌ Build failed!"
    exit 1
fi


