---
name: ui-screenshots
description: Take UI screenshots using agent-browser. Use this skill to capture visual state of UI components for code review, visual regression testing, or documentation.
metadata:
  internal: true
---

# UI Screenshots

Capture UI state for review evidence with [agent-browser](https://github.com/vercel-labs/agent-browser).
Screenshots are never committed — they are uploaded to Cloudinary and embedded in a PR comment.

## Scripts

Each script documents its own usage and requirements in its header; read it if the flags matter.

```bash
.agents/skills/ui-screenshots/scripts/check-config.sh                       # GITHUB_TOKEN, CLOUDINARY_URL, agent-browser
.agents/skills/ui-screenshots/scripts/take-screenshot.sh <URL> <OUTPUT>
.agents/skills/ui-screenshots/scripts/upload-screenshot.sh <PATH> <PR> [DESCRIPTION]
```

For anything the scripts do not cover, drive `agent-browser` directly (`open`, `screenshot --full`,
`snapshot -i -c`, `scroll`, `--session <name>` to isolate instances).

## Setup

```bash
npm install -g agent-browser
agent-browser install            # add --with-deps on Linux when system libs are missing
```

Uploading needs `CLOUDINARY_URL` (`cloudinary://API_KEY:API_SECRET@CLOUD_NAME`) and `GITHUB_TOKEN`.

## Non-obvious failures

- **Missing browser build** (e.g. `chromium_headless_shell-1208`) with `storage.googleapis.com`
  unreachable: symlink a nearby version in `/root/.cache/ms-playwright/` — minor version drift
  (1200 vs 1208) is normally compatible.
  ```bash
  cd /root/.cache/ms-playwright && ln -s chromium_headless_shell-1200 chromium_headless_shell-1208 && ln -s chromium-1200 chromium-1208
  ```
- **Page hangs on localhost**: the dev server is not up. See the local dev commands in
  [`AGENTS.md`](../../../AGENTS.md).
- **Blank screenshot**: wait for `networkidle` before capturing.
