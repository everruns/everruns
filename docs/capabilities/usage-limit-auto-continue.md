---
title: Auto-Continue After Usage Limit
description: When an LLM subscription/plan usage limit is reached, automatically resume the interrupted work shortly after the limit resets.
sidebar:
  order: 33
---

| | |
|---|---|
| **ID** | `usage_limit_auto_continue` |
| **Category** | Core |
| **Features** | None |
| **Dependencies** | None |
| **Risk** | Low |

Some providers cap usage per subscription window rather than per minute. When a
plan usage limit is hit, for example the ChatGPT/Codex `429`
`usage_limit_reached` response, the turn fails and the session goes idle until
the limit resets, often hours later. This capability makes the session resume on
its own once the window clears, so long-running work is not silently stranded.

It contributes **no tools**. The behavior is encapsulated behind a reusable
platform boundary, the capability supplies an *LLM error hook* (an in-process
capability hook, the same family as tool-call hooks and message filters) that the
agent runtime invokes generically when a turn fails with a terminal error. The
runtime has no special-casing for usage limits; any capability can provide the
same kind of error-recovery hook.

## How It Works

1. **Classification**: The provider error is classified as
   `provider_usage_limit_reached`, which captures the absolute reset time
   (`resets_at`, unix seconds) reported by the provider. This is
   driver-agnostic: any driver whose error body carries the `usage_limit_reached`
   wording is covered.
2. **Scheduling**: When the capability is enabled and a reset time is present,
   a one-shot session schedule is created to fire `delay_seconds` after the
   reset. When it fires, the configured `prompt` is injected as a user message
   and the interrupted work resumes.
3. **Message copy**: The user-facing error reads *"You're out of LLM usage
   limits. Your usage limit resets at &lt;time&gt;."* When (and only when) a
   continuation was actually scheduled, it appends *"We'll continue work
   automatically once it resets."*, so the copy never promises a resumption
   that will not happen. Without the capability, the same error stays generic
   and no continuation is scheduled.

## Configuration

### Default

```json
{
  "capabilities": ["usage_limit_auto_continue"]
}
```

Defaults: continuation fires **120 seconds** after the reported reset with the
prompt **"Continue tasks"**.

### Custom delay and prompt

```json
{
  "capabilities": [
    {
      "ref": "usage_limit_auto_continue",
      "config": {
        "delay_seconds": 300,
        "prompt": "Resume the migration where you left off."
      }
    }
  ]
}
```

| Field | Type | Default | Description |
|---|---|---|---|
| `delay_seconds` | integer (0–86400) | `120` | How long to wait after the reset before resuming. A small buffer avoids racing the provider's reset clock. |
| `prompt` | string | `"Continue tasks"` | Message injected to resume work when the limit resets. |

## When To Enable

Use this capability for agents running long, autonomous, or scheduled work on
provider plans that enforce usage-window limits, where a multi-hour stall would
otherwise require a human to manually restart the session.

## Limitations

- Requires a provider whose error reports a concrete reset time; without one the
  error copy stays generic and no continuation is scheduled.
- The continuation is delivered through the session schedule machinery, so it
  requires both a session schedule store and a schedule poller in the deployment.
  The hosted platform provides both. The default embedded runtime provides
  neither, so this capability is **not** part of the runtime-safe preset, an
  embedder must wire a `SessionScheduleStore` (e.g. via a schedule-store factory)
  and run a poller for it to take effect; otherwise its error hook is a no-op and
  the error copy stays generic.

## See Also

- [Schedules](/capabilities/session-schedules/), the scheduling machinery the continuation reuses
- [Capabilities](/features/capabilities/), the capability model
