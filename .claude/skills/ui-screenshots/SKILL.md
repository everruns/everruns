---
name: ui-screenshots
description: Take UI screenshots using Playwright and attach them as PR comments. Use this skill to capture visual state of UI components for code review, visual regression testing, or documentation.
---

# UI Screenshots Skill

Capture UI screenshots and attach them to pull requests for visual verification.

## Prerequisites

1. **UI Dependencies**: Ensure UI dependencies are installed:
   ```bash
   cd apps/ui && npm install
   ```

2. **Playwright Browser**: Chromium browser must be available:
   ```bash
   npx playwright install chromium
   ```

   In restricted environments (cloud agents), older pre-installed chromium at
   `/root/.cache/ms-playwright/chromium-1194/chrome-linux/chrome` may work better.

3. **GitHub Token**: `GITHUB_TOKEN` environment variable for PR comments.

   Screenshots are uploaded to freeimage.host and embedded directly in PR comments.

## Usage

### Taking Screenshots

Run the e2e screenshot tests:

```bash
./scripts/dev.sh e2e-screenshots
```

This captures screenshots to `apps/ui/e2e/screenshots/` (gitignored, not committed).

### Manual Screenshot Script

For custom screenshots, create a script like:

```javascript
// screenshot.mjs
import { chromium } from 'playwright';

const browser = await chromium.launch({
  executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH,
  args: ['--no-sandbox', '--disable-gpu', '--single-process'],
});

const page = await browser.newPage();
await page.goto('http://localhost:9100/dev/components');
await page.waitForLoadState('networkidle');
await page.screenshot({ path: 'screenshot.png', fullPage: true });
await browser.close();
```

Run with:
```bash
PLAYWRIGHT_CHROMIUM_PATH=/root/.cache/ms-playwright/chromium-1194/chrome-linux/chrome \
  node screenshot.mjs
```

### Attaching Screenshots to PR

Screenshots are NOT committed to the repo. They are uploaded to freeimage.host and embedded in PR comments:

```bash
# Use the helper script
.claude/skills/ui-screenshots/scripts/upload-screenshot.sh \
  apps/ui/e2e/screenshots/dev-components-full.png \
  195  # PR number
```

## Integration with Smoke Tests

The smoke test skill can call this skill to capture UI state:

1. Run e2e screenshot tests as part of smoke testing
2. If a PR branch is detected, upload screenshots and add PR comment
3. Screenshots help reviewers verify visual changes

## Troubleshooting

### Browser crashes in restricted environments

Use the `--single-process` flag and specify an older chromium:

```bash
export PLAYWRIGHT_CHROMIUM_PATH=/root/.cache/ms-playwright/chromium-1194/chrome-linux/chrome
```

### Page hangs on localhost

The dev server may not be running. Start it first:

```bash
cd apps/ui && npm run dev &
sleep 10  # Wait for server
```

Or run tests with webServer config (in playwright.config.ts).

### Permission denied for /tmp

In sandboxed environments, shared memory may fail. Use `--disable-dev-shm-usage` flag.

### Image upload fails

The script uses freeimage.host for image hosting. If uploads fail, check network connectivity.

## Available Screenshots

The e2e tests capture these screenshots (stored locally, not in repo):

| Screenshot | Description |
|------------|-------------|
| `dev-components-full.png` | Full page of dev components showcase |
| `dev-components-messages.png` | Message rendering section |
| `dev-components-toolcalls.png` | Tool call cards section |

## Script Reference

### take-screenshot.sh

Take a screenshot of a URL:

```bash
.claude/skills/ui-screenshots/scripts/take-screenshot.sh [URL] [OUTPUT_PATH]
```

Example:
```bash
.claude/skills/ui-screenshots/scripts/take-screenshot.sh \
  http://localhost:9100/dev/components \
  apps/ui/e2e/screenshots/custom.png
```

### upload-screenshot.sh

Upload screenshot to freeimage.host and add PR comment:

```bash
.claude/skills/ui-screenshots/scripts/upload-screenshot.sh <SCREENSHOT_PATH> <PR_NUMBER> [DESCRIPTION]
```

Example:
```bash
.claude/skills/ui-screenshots/scripts/upload-screenshot.sh \
  apps/ui/e2e/screenshots/dev-components-full.png \
  195 \
  "Dev components page showing message and tool call rendering"
```
