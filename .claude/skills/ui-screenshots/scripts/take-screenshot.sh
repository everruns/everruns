#!/usr/bin/env bash
set -euo pipefail

# Take a screenshot of a URL using Playwright
#
# Usage: take-screenshot.sh <URL> <OUTPUT_PATH>
#
# Example:
#   ./take-screenshot.sh http://localhost:9100/dev/components screenshot.png

URL="${1:-http://localhost:9100/dev/components}"
OUTPUT_PATH="${2:-screenshot.png}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

# Find chromium - prefer older version that works in restricted environments
CHROMIUM_PATHS=(
  "/root/.cache/ms-playwright/chromium-1194/chrome-linux/chrome"
  "/root/.cache/ms-playwright/chromium-1200/chrome-linux64/chrome"
)

CHROMIUM_PATH=""
for path in "${CHROMIUM_PATHS[@]}"; do
  if [ -f "$path" ]; then
    CHROMIUM_PATH="$path"
    break
  fi
done

if [ -z "$CHROMIUM_PATH" ]; then
  echo "❌ Chromium not found. Install with: npx playwright install chromium"
  exit 1
fi

echo "📸 Taking screenshot of $URL"
echo "   Using chromium: $CHROMIUM_PATH"
echo "   Output: $OUTPUT_PATH"

# Run the script from apps/ui directory where playwright is installed
cd "$PROJECT_ROOT/apps/ui"

# Create temporary script in current directory (where node_modules exists)
TEMP_SCRIPT=$(mktemp ./screenshot-XXXXXX.mjs)
cat > "$TEMP_SCRIPT" << EOF
import { chromium } from 'playwright';

const browser = await chromium.launch({
  executablePath: '$CHROMIUM_PATH',
  args: [
    '--no-sandbox',
    '--disable-setuid-sandbox',
    '--disable-gpu',
    '--disable-software-rasterizer',
    '--disable-dev-shm-usage',
    '--single-process',
  ],
});

const page = await browser.newPage();

try {
  await page.goto('$URL', { waitUntil: 'networkidle', timeout: 30000 });
  await page.waitForTimeout(2000);
  await page.screenshot({ path: '$OUTPUT_PATH', fullPage: true });
  console.log('✅ Screenshot saved to $OUTPUT_PATH');
} catch (e) {
  console.error('❌ Screenshot failed:', e.message);
  process.exit(1);
} finally {
  await browser.close();
}
EOF

# Run the script (already in apps/ui directory)
node "$TEMP_SCRIPT"
rm -f "$TEMP_SCRIPT"
