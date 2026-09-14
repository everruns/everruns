---
name: ui-screenshots
description: Take UI screenshots using agent-browser. Use this skill to capture visual state of UI components for code review, visual regression testing, or documentation.
metadata:
  internal: true
---

# UI Screenshots

Capture UI state for review evidence with [agent-browser](https://github.com/vercel-labs/agent-browser).
Screenshots are never committed — the helper uploads them to GitHub's user-attachments CDN and
embeds them in a PR comment. The general media and video rules live in
[`../ship/references/pr-and-merge.md`](../ship/references/pr-and-merge.md#publish-evidence-assets).

## Scripts

Each script documents its own usage and requirements in its header; read it if the flags matter.

```bash
.agents/skills/ui-screenshots/scripts/check-config.sh                       # GitHub auth, agent-browser
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

Uploading needs `GITHUB_TOKEN` or an authenticated `gh` CLI session. The token must have push access
to `everruns/everruns`.

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
- **Every HTTPS page fails with `ERR_CONNECTION_RESET` from a cloud agent**, while `curl` through
  the same proxy works: Chrome's TLS 1.3 ClientHello carries a post-quantum key share that pushes it
  to ~2 KB, and the egress relay accepts the `CONNECT` then cuts the tunnel mid-handshake. `curl`
  and `openssl s_client` never offer a PQ key share, which is why the proxy tests healthy by hand.
  `take-screenshot.sh` already passes `--ssl-version-max=tls1.2`; when driving `agent-browser`
  yourself, export the flags instead of passing them per command (EVE-807):
  ```bash
  export AGENT_BROWSER_ARGS=$'--ssl-version-max=tls1.2\n--disable-features=PostQuantumKyber'
  ```
  Confirm they actually reached Chrome — a daemon relaunch silently drops flags given only on the
  first `open`, and the symptom is indistinguishable from the bug itself:
  ```bash
  pgrep -a chrome | tr ' ' '\n' | grep ssl-version
  ```
  Use `AGENT_BROWSER_ARGS` rather than `--args`: `--args` splits on commas, so a multi-value flag
  like `--disable-features=A,B` becomes two argv entries and Chrome exits with
  `Multiple targets are not supported in headless mode`.
