---
name: manual-ui-testing
description: Run manual UI test cases using agent-browser against a running stack. Use when the user asks to run UI tests, test the UI, run manual tests, or verify UI behavior.
metadata:
  internal: true
user-invocable: true
allowed-tools: Bash(npx agent-browser:*), Bash(agent-browser:*), Bash(just:*), Bash(doppler:*)
---

# Manual UI Testing

Execute the test cases in `test_cases/ui/` against a running stack with `agent-browser`, record
results, and file issues for failures. [`knowledge/evaluation/test-cases.md`](../../../knowledge/evaluation/test-cases.md) defines
the test case format.

## Scope

Each subdirectory of `test_cases/ui/` is a category — list the directory rather than assuming a
fixed set. Every case states its own preconditions (auth mode, existing data), test data, steps, and
expected result; read them before running. With no scope given, run everything in dependency order:
auth → org → features.

## Stack

Full auth mode needs the real stack, not DEV_MODE. Start it with a unique per-worktree prefix and
wait for PostgreSQL, Valkey, API, worker, UI, and Caddy to come up:

```bash
PORT_PREFIX=<prefix> doppler run -- just start-all
curl -s http://localhost:<prefix>00/healthz
```

If a stack is already running, confirm its `PORT_PREFIX` and auth mode before using it.

## Driving the browser

`agent-browser` runs headless Chromium via `npx`; the daemon persists between commands in a session.

```bash
agent-browser open http://localhost:<prefix>00/<path>
agent-browser wait --load networkidle
agent-browser snapshot -i            # → @e1 [input type="email"], @e2 [button] "Submit", …
agent-browser fill @e1 "value"
agent-browser click @e2
agent-browser wait --load networkidle
agent-browser snapshot -i
agent-browser screenshot /tmp/test_<category>_<tc>.png
```

Hints that cost time to rediscover:

- Refs are invalidated by any navigation or DOM change — re-snapshot after every one.
- Always `wait --load networkidle` after navigation, form submission, and before screenshots.
- Chain independent commands with `&&`; do not chain when the next ref depends on reading a snapshot.
- Next.js dev compilation can add 2–5s to a first page load.
- Keyboard shortcuts (Ctrl+K) often do not reach headless Chromium — drive the equivalent click.
- Screenshot each significant step, not just the verdict.
- Element missing from a snapshot? `agent-browser scroll down` first.
- Login redirect loop usually means `AUTH_MODE` does not match the test category.

## Recording

Write or update `test_cases/ui/MANUAL_TEST_RESULTS_<YYYY-MM-DD>.md` using
[`references/results-template.md`](references/results-template.md). Partial or re-test runs append to
(or update) the existing file for that date.

If the user asks for issues to be filed, create them via Linear MCP (EVE team, OSS project) with
severity, repro steps, expected vs actual, and the test case ID (e.g. `org_creation/TC003`).
