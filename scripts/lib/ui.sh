#!/usr/bin/env bash
# UI operations: dev, build, install, e2e, screenshots

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cmd="${1:-}"
UI_DIR="$PROJECT_ROOT/apps/ui"

# Set chromium path for restricted environments
setup_playwright() {
  CHROMIUM_PATH="/root/.cache/ms-playwright/chromium-1194/chrome-linux/chrome"
  if [ -f "$CHROMIUM_PATH" ]; then
    export PLAYWRIGHT_CHROMIUM_PATH="$CHROMIUM_PATH"
  fi
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
    echo "✅ UI dependencies installed!"
    ;;

  e2e)
    echo "🎭 Running UI e2e tests..."
    cd "$UI_DIR"
    setup_playwright
    npm run e2e
    echo "✅ E2E tests complete!"
    ;;

  screenshots)
    echo "📸 Taking UI screenshots..."
    cd "$UI_DIR"
    setup_playwright
    npm run e2e:screenshots
    echo "✅ Screenshots saved to apps/ui/e2e/screenshots/"
    ;;

  *)
    echo "Usage: $0 {dev|build|install|e2e|screenshots}"
    exit 1
    ;;
esac
