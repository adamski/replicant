#!/bin/bash
set -e

# Build distribution headers and libraries
# This script creates a stable SDK in the dist/ folder

echo "Building sync-client library..."
cargo build --package sync-client --release

echo "Creating dist directory structure..."
mkdir -p dist/include
mkdir -p dist/lib
mkdir -p dist/examples
mkdir -p dist/cmake

echo "Copying generated C header to dist..."
if [ -f "sync-client/target/include/sync_client.h" ]; then
    cp sync-client/target/include/sync_client.h dist/include/
    echo "✓ C header copied to dist/include/sync_client.h"
else
    echo "✗ Generated C header not found. Make sure cargo build completed successfully."
    exit 1
fi

echo "Copying built libraries to dist..."
if [ -f "target/release/libsync_client.a" ]; then
    cp target/release/libsync_client.a dist/lib/
    echo "✓ Static library copied to dist/lib/"
fi

if [ -f "target/release/libsync_client.dylib" ]; then
    cp target/release/libsync_client.dylib dist/lib/
    echo "✓ Dynamic library copied to dist/lib/"
fi

# For Linux
if [ -f "target/release/libsync_client.so" ]; then
    cp target/release/libsync_client.so dist/lib/
    echo "✓ Shared library copied to dist/lib/"
fi

echo "Adding version information..."
CARGO_VERSION=$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "sync-client") | .version')
echo "# Sync Client SDK v$CARGO_VERSION" > dist/VERSION.md
echo "" >> dist/VERSION.md
echo "Generated on: $(date)" >> dist/VERSION.md
echo "Rust version: $(rustc --version)" >> dist/VERSION.md

echo ""
echo "✓ Distribution build complete!"
echo "SDK available in dist/ folder:"
echo "  - dist/include/sync_client.h     (C header)"
echo "  - dist/include/sync_client.hpp   (C++ header - create manually)"
echo "  - dist/lib/                      (compiled libraries)"
echo "  - dist/examples/                 (usage examples)"
echo ""
echo "Version: $CARGO_VERSION"