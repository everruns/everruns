# Desktop X review

An Everruns agent that opens a browser on a Sandbox desktop and reviews the
recent posts of an X account. It is the end-to-end check for desktops in
Sandbox Templates: a real model (`gpt-6.1-sol`) drives a real Daytona or E2B
desktop through the `computer` tool, with no scripted steps.

The agent definition is [`desktop-x-review.md`](desktop-x-review.md). The
runner, [`run.py`](run.py), needs only the Python standard library.

## What it shows

- **One machine.** The Sandbox Template turns on `desktop`, so the agent's
  shell, files and screen are the same Sandbox. It starts the browser from the
  shell and reads the page through screenshots.
- **No capability to pick.** The agent lists no computer-use capability; the
  template adds `sandbox_desktop` itself.
- **Disposable compute.** The template deletes the Sandbox once it has been
  idle for ten minutes. The Session and its records stay.

## Run it

```bash
# 1. Start Everruns (see AGENTS.md, "Local dev").
PORT_PREFIX=271 AUTH_MODE=none ./scripts/start-agent-dev.sh

# 2. Review @everrunshq on a Daytona desktop.
DAYTONA_API_KEY=... python3 examples/desktop-x-review/run.py

# Or on E2B, or another account:
E2B_API_KEY=... python3 examples/desktop-x-review/run.py --provider e2b
python3 examples/desktop-x-review/run.py --handle rustlang
```

The first run saves the provider key as an organization account named
`Desktop X review (daytona)` (or `e2b`) and creates the agent; later runs reuse
both. `EVERRUNS_API_URL` (default `http://localhost:27100/api`),
`EVERRUNS_API_KEY`, `EVERRUNS_ORG` and `EVERRUNS_MODEL` (default
`gpt-6.1-sol`) point it elsewhere.

## Expected outcome

The runner prints each tool call as it happens, then the review: the account's
name, bio and follower count, a newest-first list of recent posts, themes, and
two or three suggestions. The same review is saved as `x-review.md` in the
Sandbox workspace.

```text
  [bash] (chromium --no-sandbox --test-type --start-maximized https://x.com/everrunshq || ...
  [computer] screenshot
  [computer] scroll at [791, 611]
  [computer] scroll at [785, 448]
  ...
  [write_file] /workspace/x-review.md

Agent:
# X review: @everrunshq

## Account
- **Name:** everruns (verified badge shown)
- **Bio:** "Durable agentic harness engine - everruns.com"
...
```

## Limits

- X shows logged-out visitors a sign-in overlay and a partial timeline. The
  agent never signs in, so the review covers what the public page shows and
  says where it could not read.
- Each action returns a screenshot, which costs image tokens. A run takes
  a few minutes and dozens of model calls.
- Provider charges apply for the Sandbox while it runs.
