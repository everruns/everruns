---
title: Handle errors and cancel turns
description: Detect and recover from failed turns, cancel a long-running turn, and react to common error events from the SSE stream.
appliesTo: [platform, cloud]
---

Turns can fail (the LLM rejected the request, a tool errored repeatedly) or be cancelled by the user. The event stream tells you which. This guide covers both paths.

## Detect a failed turn

Failures surface as `turn.failed` events:

```python
async for event in client.events.stream(session.id):
    if event.type == "turn.completed":
        break
    if event.type == "turn.failed":
        message = event.data.get("error", "unknown")
        code = event.data.get("error_code")
        print(f"Turn failed [{code}]: {message}")
        break
```

`error` is a human-readable string. `error_code` is a stable, machine-readable string; branch on it, not on the message text. Use it to decide whether to retry, surface to the user, or escalate:

```python
RETRYABLE = {"provider_rate_limited", "provider_unavailable", "dependency_unavailable"}

if event.type == "turn.failed":
    code = event.data.get("error_code")
    if code in RETRYABLE:
        ...  # wait, then retry with backoff
    elif code in {"budget_exhausted", "budget_paused"}:
        ...  # raise or resume the budget
    else:
        ...  # surface the message to the user
```

With the `generic` error disclosure mode, the displayed `error_code` collapses to `processing_error` to hide provider detail.

## Cancel a long-running turn

To cancel a turn that's already running:

```python
import asyncio

await client.messages.create(session.id, "Analyse every Python package on PyPI")
await asyncio.sleep(2)
await client.sessions.cancel(session.id)
```

Cancellation emits a `turn.cancelled` event, appends a user message noting the cancellation, and the worker emits a final agent message confirming the work was stopped. The session itself stays open and accepts new messages.

## React to all three terminal states

```python
TERMINAL = {"turn.completed", "turn.failed", "turn.cancelled"}

async for event in client.events.stream(session.id):
    if event.type in TERMINAL:
        print(f"[{event.type}]")
        if event.type == "turn.failed":
            print(event.data.get("error"))
        break
```

## Retry a failed turn

Failed turns don't auto-retry from the application's perspective (durable execution retries individual steps inside a turn, not the whole turn). To retry, send the message again:

```python
async def send_with_retry(client, session_id, content, attempts=2):
    for attempt in range(attempts):
        await client.messages.create(session_id, content)
        async for event in client.events.stream(session_id):
            if event.type == "turn.completed":
                return True
            if event.type == "turn.failed":
                if attempt == attempts - 1:
                    return False
                break
    return False
```

Don't retry indefinitely, a turn that fails twice usually fails for a reason (rate limit, malformed prompt, missing capability). Surface to the user.

## Common error codes

| `error_code` | Cause | Action |
|---|---|---|
| `provider_rate_limited` | LLM provider throttled the request | Wait, retry with backoff |
| `provider_usage_limit_reached` | Provider subscription or plan limit hit; resets at a later time | Wait for the reset time, or switch model |
| `provider_unavailable` | Provider is down or unreachable | Retry later, or switch provider |
| `provider_quota_exhausted` | Provider account is out of credits | Top up the provider account |
| `provider_misconfigured` | Missing or invalid provider API key | Fix the provider key |
| `provider_attestation_required` | Provider needs an account confirmation (for example OpenRouter age verification) | Complete it in the provider settings |
| `model_unavailable` / `model_not_configured` | Model missing, or none set for the agent or session | Pick a configured model |
| `request_too_large` | Context overflowed even after compaction | Trim the conversation, start a fresh session |
| `invalid_tool_schema` | A tool definition was rejected by the provider | Fix the tool's schema |
| `budget_exhausted` / `budget_paused` | The budget has no room left, or is paused | Raise or resume the budget, see [Enforce a budget](/how-to/enforce-a-budget/) |
| `blocked_by_hook` | A `user_prompt_submit` hook rejected the message | Change the message or the hook |
| `processing_error` | Unclassified failure, or details hidden by disclosure mode | Check server logs |

Cancellation is not a failure: `sessions.cancel` produces `turn.cancelled`, which needs no retry.

The code list lives in `crates/contracts/src/user_facing_error.rs`.

## See also

- [Stream events](/how-to/stream-events/)
- [Event Reference](/event-reference/), all event types and payloads.
- [The agentic loop](/explanation/agentic-loop/#what-happens-when-the-loop-gets-stuck), failure modes.
