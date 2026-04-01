#!/usr/bin/env bash
# UI operations: dev, build, install, e2e

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cmd="${1:-}"
UI_DIR="$PROJECT_ROOT/apps/ui"

# Set chromium path for restricted environments
# Can be overridden via PLAYWRIGHT_CHROMIUM_PATH environment variable
setup_playwright() {
  # Use environment variable if set, otherwise try common locations
  if [ -n "${PLAYWRIGHT_CHROMIUM_PATH:-}" ] && [ -f "$PLAYWRIGHT_CHROMIUM_PATH" ]; then
    export PLAYWRIGHT_CHROMIUM_PATH
    return
  fi

  # Try common Playwright chromium locations
  local chromium_paths=(
    "/root/.cache/ms-playwright/chromium-1194/chrome-linux/chrome"
    "$HOME/.cache/ms-playwright/chromium-1194/chrome-linux/chrome"
    "/home/user/.cache/ms-playwright/chromium-1194/chrome-linux/chrome"
  )

  for path in "${chromium_paths[@]}"; do
    if [ -f "$path" ]; then
      export PLAYWRIGHT_CHROMIUM_PATH="$path"
      return
    fi
  done
}

case "$cmd" in
  dev)
    echo "🖥️  Starting UI development server..."
    cd "$UI_DIR"
    npm run dev
    ;;

  build)
    echo "🔨 Building UI for production..."
    cd "$UI_DIR"
    npm run build
    echo "✅ UI build complete!"
    ;;

  install)
    echo "📦 Installing UI dependencies..."
    cd "$UI_DIR"
    npm install
    npm rebuild
    echo "✅ UI dependencies installed!"
    ;;

  e2e)
    echo "🎭 Running UI e2e tests..."
    cd "$UI_DIR"
    setup_playwright
    npm run e2e
    echo "✅ E2E tests complete!"
    ;;

  *)
    echo "Usage: $0 {dev|build|install|e2e}"
    exit 1
    ;;
esac
