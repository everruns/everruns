#!/usr/bin/env bash
set -euo pipefail

# Install agent-browser and its dependencies
#
# Usage: install-browser.sh [--with-deps]
#
# Options:
#   --with-deps  Also install system dependencies (Linux only)

WITH_DEPS=false
for arg in "$@"; do
  case $arg in
    --with-deps)
      WITH_DEPS=true
      ;;
  esac
done

echo "🔧 Installing agent-browser..."
echo ""

# Check if npm is available
if ! command -v npm &> /dev/null; then
  echo "❌ npm not found. Please install Node.js first."
  exit 1
fi

# Install agent-browser globally
echo "📦 Installing agent-browser package..."
npm install -g agent-browser

# Verify installation
if ! command -v agent-browser &> /dev/null; then
  echo "❌ agent-browser installation failed"
  exit 1
fi

echo "✅ agent-browser installed"
echo ""

# Install browser with dependencies if requested
if [ "$WITH_DEPS" = true ]; then
  echo "📦 Installing Chromium with system dependencies..."
  agent-browser install --with-deps
else
  echo "📦 Installing Chromium..."
  agent-browser install
fi

echo ""
echo "✅ Installation complete!"
echo ""
echo "Verify with: agent-browser --version"
