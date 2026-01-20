#!/usr/bin/env bash
# Docs operations: dev, build, install

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cmd="${1:-}"
DOCS_DIR="$PROJECT_ROOT/apps/docs"

case "$cmd" in
  dev)
    echo "📚 Starting docs development server..."
    cd "$DOCS_DIR"
    npm run dev
    ;;

  build)
    echo "🔨 Building docs for production..."
    cd "$DOCS_DIR"
    npm run check && npm run build
    echo "✅ Docs build complete!"
    ;;

  install)
    echo "📦 Installing docs dependencies..."
    cd "$DOCS_DIR"
    npm install
    echo "✅ Docs dependencies installed!"
    ;;

  *)
    echo "Usage: $0 {dev|build|install}"
    exit 1
    ;;
esac
