#!/bin/bash
# Install git hooks for the project

set -e

HOOKS_DIR=".git/hooks"
SCRIPTS_DIR="scripts/hooks"

echo "Installing git hooks..."

# Check if we're in the workspace root
if [ ! -d "$SCRIPTS_DIR" ]; then
    echo "❌ Error: Must run this script from the workspace root directory"
    exit 1
fi

# Create hooks directory if it doesn't exist
mkdir -p "$HOOKS_DIR"

# Install pre-commit hook
if [ -f "$HOOKS_DIR/pre-commit" ]; then
    echo "⚠️  Pre-commit hook already exists, backing up to pre-commit.backup"
    cp "$HOOKS_DIR/pre-commit" "$HOOKS_DIR/pre-commit.backup"
fi

cp "$SCRIPTS_DIR/pre-commit" "$HOOKS_DIR/pre-commit"
chmod +x "$HOOKS_DIR/pre-commit"

echo "✅ Git hooks installed successfully"
echo ""
echo "Pre-commit hook will now run 'cargo fmt --all' before each commit"
echo "This ensures all committed code is properly formatted"
