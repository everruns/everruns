---
type: Test Case
title: "TC003: Sessions - Hook-Rewritten Tool Arguments"
description: "Verifies that a tool call whose arguments a pre_tool_use hook rewrote shows an indicator and the executed arguments beside the model-authored ones, and that unrewritten calls show nothing extra."
tags:
  - everruns
  - test-case
  - ui
  - sessions
---
# TC003: Sessions - Hook-Rewritten Tool Arguments

## Description

Verifies that a tool call whose arguments a pre_tool_use hook rewrote shows an indicator and the
executed arguments beside the model-authored ones, and that unrewritten calls show nothing extra.

## Preconditions

- The API, worker, UI, and reverse proxy are running with `AUTH_MODE=none` or an authenticated user.
- An agent with the `user_hooks` capability and a `pre_tool_use` hook on `bash` that returns a
  `mutate` decision rewriting `command` (see `docs/capabilities/user-hooks.md`).
- The agent can also call a tool the hook does not match.

## Test Data

| Field | Value |
| --- | --- |
| Rewritten call | `bash` with `command: "rm -rf build"`, hook rewrites to `rm -rf ./build` |
| Unrewritten call | Any tool the hook does not match |
| Oversized rewrite | Hook output whose arguments exceed 8 KiB |

## Steps

1. Start a session and ask the agent to run the rewritten call, then the unrewritten call.
2. Open the session Transcript and expand the turn's tool activity.
3. Click "Arguments rewritten by hook" on the rewritten call.
4. Inspect the unrewritten call.
5. Repeat step 1 with the oversized rewrite and expand its indicator.

## Expected Result

- The rewritten call shows an "Arguments rewritten by hook" indicator without expanding anything.
- Expanding it shows "Original arguments" (model-authored) and "Executed arguments" (what ran);
  credential-named values read `[REDACTED]`.
- The unrewritten call shows no indicator.
- The oversized rewrite shows the truncated JSON preview verbatim with a truncated-preview note.
