---
name: ui-screenshots
description: Take UI screenshots using agent-browser and attach them as PR comments. Use this skill to capture visual state of UI components for code review, visual regression testing, or documentation.
---

# UI Screenshots Skill

Capture UI screenshots and attach them to pull requests for visual verification.

Uses [agent-browser](https://github.com/vercel-labs/agent-browser) for screenshots (fast Rust CLI with Node.js fallback). Playwright is still used for e2e tests.

## Prerequisites

1. **agent-browser**: Install the browser automation CLI:
   ```bash
   npm install -g agent-browser
   agent-browser install  # Download Chromium
   ```

   For Linux with missing dependencies:
   ```bash
   agent-browser install --with-deps
   ```

2. **Cloudinary Account** (free tier available):
   - Create account at [cloudinary.com](https://cloudinary.com)
   - Get your `CLOUDINARY_URL` from the dashboard (format: `cloudinary://API_KEY:API_SECRET@CLOUD_NAME`)
   - Set environment variable: `CLOUDINARY_URL`

3. **GitHub Token**: `GITHUB_TOKEN` environment variable for PR comments.

## Usage

### Taking Screenshots

Use the take-screenshot script:

```bash
.claude/skills/ui-screenshots/scripts/take-screenshot.sh [URL] [OUTPUT_PATH]
```

Example:
```bash
.claude/skills/ui-screenshots/scripts/take-screenshot.sh \
  http://localhost:9100/dev/components \
  screenshot.png
```

### Using agent-browser directly

For more control, use agent-browser CLI commands:

```bash
# Open a page
agent-browser open http://localhost:9100/dev/components

# Take full-page screenshot
agent-browser screenshot screenshot.png --full

# Get accessibility snapshot (useful for AI agents)
agent-browser snapshot -i -c
```

### Running e2e Screenshot Tests

The Playwright e2e tests still work for automated screenshot capture:

```bash
./scripts/dev.sh e2e-screenshots
```

This captures screenshots to `apps/ui/e2e/screenshots/` (gitignored, not committed).

### Attaching Screenshots to PR

Screenshots are NOT committed to the repo. They are uploaded to Cloudinary and embedded in PR comments:

```bash
.claude/skills/ui-screenshots/scripts/upload-screenshot.sh \
  screenshot.png \
  195  # PR number
```

## Integration with Smoke Tests

The smoke test skill can call this skill to capture UI state:

1. Run screenshot capture as part of smoke testing
2. If a PR branch is detected, upload screenshots and add PR comment
3. Screenshots help reviewers verify visual changes

## Troubleshooting

### agent-browser not found

Install globally:
```bash
npm install -g agent-browser
agent-browser install
```

### Missing browser dependencies on Linux

```bash
agent-browser install --with-deps
# Or manually:
npx playwright install-deps chromium
```

### Page hangs on localhost

The dev server may not be running. Start it first:

```bash
cd apps/ui && npm run dev &
sleep 10  # Wait for server
```

### Session conflicts

agent-browser uses sessions to isolate browser instances:
```bash
# Use a named session
agent-browser --session screenshots open http://localhost:9100

# List sessions
agent-browser session list
```

### Cloudinary upload fails

Verify `CLOUDINARY_URL` is set correctly:
- Format: `cloudinary://API_KEY:API_SECRET@CLOUD_NAME`
- Get from Cloudinary dashboard

## Available Screenshots

The e2e tests capture these screenshots (stored locally, not in repo):

| Screenshot | Description |
|------------|-------------|
| `message-components-full.png` | Full page of message components showcase |
| `message-components-messages.png` | Message rendering section |
| `message-components-toolcalls.png` | Tool call cards section |

## Script Reference

### take-screenshot.sh

Take a screenshot of a URL using agent-browser:

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

Upload screenshot to Cloudinary and add PR comment:

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

### check-config.sh

Check if cloud agent environment is configured:

```bash
.claude/skills/ui-screenshots/scripts/check-config.sh
```

Verifies: `GITHUB_TOKEN`, `CLOUDINARY_URL`, and agent-browser availability.

## agent-browser Commands Reference

Common commands for screenshot workflows:

| Command | Description |
|---------|-------------|
| `agent-browser open <url>` | Navigate to a URL |
| `agent-browser screenshot [path]` | Capture screenshot |
| `agent-browser screenshot path --full` | Full page screenshot |
| `agent-browser snapshot` | Get accessibility tree (for AI) |
| `agent-browser snapshot -i -c` | Interactive elements only, compact |
| `agent-browser click <selector>` | Click element |
| `agent-browser wait <selector>` | Wait for element |
| `agent-browser scroll down` | Scroll page |
| `agent-browser --json ...` | JSON output for automation |

See [agent-browser docs](https://github.com/vercel-labs/agent-browser) for full reference.
