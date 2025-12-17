#!/bin/bash
set -e

# Build distribution headers and libraries
# This script creates a stable SDK in the dist/ folder

echo "Building replicant-client library..."
cargo build --package replicant-client --release

echo "Creating dist directory structure..."
mkdir -p dist/include
mkdir -p dist/lib
mkdir -p dist/examples
mkdir -p dist/cmake

echo "Copying generated C header to dist..."
if [ -f "replicant-client/target/include/replicant.h" ]; then
    cp replicant-client/target/include/replicant.h dist/include/
    echo "✓ C header copied to dist/include/replicant.h"
else
    echo "✗ Generated C header not found. Make sure cargo build completed successfully."
    exit 1
fi

echo "Copying C++ wrapper header to dist..."
if [ -f "replicant-client/include/replicant.hpp" ]; then
    cp replicant-client/include/replicant.hpp dist/include/
    echo "✓ C++ header copied to dist/include/replicant.hpp"
else
    echo "⚠ C++ header not found at replicant-client/include/replicant.hpp"
fi

echo "Copying built libraries to dist..."
if [ -f "target/release/libreplicant_client.a" ]; then
    cp target/release/libreplicant_client.a dist/lib/
    echo "✓ Static library copied to dist/lib/"
fi

if [ -f "target/release/libreplicant_client.dylib" ]; then
    cp target/release/libreplicant_client.dylib dist/lib/
    echo "✓ Dynamic library copied to dist/lib/"
fi

# For Linux
if [ -f "target/release/libreplicant_client.so" ]; then
    cp target/release/libreplicant_client.so dist/lib/
    echo "✓ Shared library copied to dist/lib/"
fi

echo "Copying JUCE module to dist..."
if [ -d "wrappers/juce/replicant" ]; then
    rm -rf dist/juce/replicant
    mkdir -p dist/juce/replicant

    cp wrappers/juce/replicant/replicant.h dist/juce/replicant/
    cp wrappers/juce/replicant/replicant.cpp dist/juce/replicant/

    # Fix include path for dist layout
    sed -i.bak 's|"../../../dist/include/replicant.hpp"|"../../include/replicant.hpp"|g' dist/juce/replicant/replicant.h
    rm -f dist/juce/replicant/replicant.h.bak

    echo "✓ JUCE module copied to dist/juce/replicant/"
else
    echo "⚠ JUCE module not found at wrappers/juce/replicant/"
fi

echo "Adding version information..."
CARGO_VERSION=$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "replicant-client") | .version')
echo "# Replicant SDK v$CARGO_VERSION" > dist/VERSION.md
echo "" >> dist/VERSION.md
echo "Generated on: $(date)" >> dist/VERSION.md
echo "Rust version: $(rustc --version)" >> dist/VERSION.md

echo ""
echo "✓ Distribution build complete!"
echo "SDK available in dist/ folder:"
echo "  - dist/include/replicant.h     (C header - auto-generated)"
echo "  - dist/include/replicant.hpp   (C++ wrapper)"
echo "  - dist/lib/                    (compiled libraries)"
echo "  - dist/juce/replicant/         (JUCE module)"
echo "  - dist/examples/               (usage examples)"
echo ""
echo "Version: $CARGO_VERSION"
