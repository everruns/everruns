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

Cancellation records `turn.cancelled` and a system-origin cancellation message. A live worker records the cancellation; when no worker owns the turn, the control plane records it and settles the session. Do not depend on a final agent message to detect cancellation. The session itself stays open and accepts new messages.

## React to terminal states

```python
TERMINAL = {"turn.completed", "turn.failed", "turn.cancelled", "turn.sealed"}

async for event in client.events.stream(session.id):
    if event.type in TERMINAL:
        print(f"[{event.type}]")
        if event.type == "turn.failed":
            print(event.data.get("error"))
        break
```

`turn.sealed` means the runtime stopped a turn to prevent repeated no-progress recovery or exhausted work budget; it is not successful completion.

## Retry a failed turn

Crash recovery resumes an open turn from its journal; it is not a new user message. Some provider failures also schedule automatic recovery, indicated by `auto_retry` or `credit_wait` under `event.data["error_fields"]` (see the table below). Wait for that recovery instead of submitting duplicate work. For a failed turn without automatic recovery, send the message again only after checking whether earlier tools already produced external effects:

Use this example only with an idle session that has a single message writer, and only when repeating the work is safe. `messages.create` returns a message, not an event cursor: retrieve its persisted `input.message` before subscribing. Filtering to durable terminal events also keeps ephemeral delta IDs out of reconnection cursors.

```python
import asyncio
from contextlib import aclosing

RETRYABLE = {"provider_rate_limited", "provider_unavailable", "dependency_unavailable"}
TERMINAL = ["turn.completed", "turn.failed", "turn.cancelled", "turn.sealed"]

async def send_with_retry(client, session_id, content, attempts=2, timeout=120):
    try:
        async with asyncio.timeout(timeout):  # Python 3.11+, total wait budget
            for attempt in range(attempts):
                message = await client.messages.create(session_id, content)
                inputs = await client.events.list(
                    session_id, types=["input.message"], order_desc=True, limit=1
                )
                if not inputs or inputs[0].data.get("message", {}).get("id") != message.id:
                    raise RuntimeError("Session has another writer; inspect before retrying")
                stream = client.events.stream(
                    session_id, since_id=inputs[0].id, types=TERMINAL
                )
                async with aclosing(stream):
                    async for event in stream:
                        if event.type == "turn.completed":
                            return True
                        if event.type in {"turn.cancelled", "turn.sealed"}:
                            return False
                        if event.type == "turn.failed":
                            fields = event.data.get("error_fields") or {}
                            if fields.get("auto_retry") or fields.get("credit_wait"):
                                continue  # recovery is already scheduled
                            if event.data.get("error_code") not in RETRYABLE:
                                return False
                            if attempt == attempts - 1:
                                return False
                            break
                    else:
                        return None  # stream ended; outcome unknown, do not resend
                await asyncio.sleep(2 ** attempt)
    except TimeoutError:
        return None  # still running or recovering; inspect, don't submit again
    return False
```

`True` means completed, `False` means stopped without successful completion, and `None` means the outcome is unknown. A timeout stops waiting, not the server-side turn or scheduled recovery. Inspect the session before taking further action. Do not retry indefinitely.

## Common error codes

| `error_code` | Cause | Action |
|---|---|---|
| `provider_rate_limited` | LLM provider throttled the request. `reason: "in_flight_budget_exhausted"` means the provider's in-flight spending limit is reached; `retry_after` is the wait in seconds | Wait, retry with backoff. When `retry_after` is at most 600 s, the session retries on its own (`auto_retry: true`), up to 3 times an hour |
| `provider_usage_limit_reached` | Provider subscription or plan limit hit; resets at a later time | Wait for the reset time, or switch model |
| `provider_unavailable` | Provider is down or unreachable | Retry later, or switch provider |
| `provider_quota_exhausted` | Provider account is out of credits. `requested_max_tokens` and `affordable_tokens` are set when the balance covers fewer output tokens than requested | Top up the provider account, or lower max output tokens. The runtime first retries once with fewer max output tokens when the balance covers at least 4,096; otherwise the session checks for credits every minute for up to an hour and continues on its own (`credit_wait: true`), see [`turn.credit_shortfall`](/event-reference/#turncredit_shortfall) |
| `provider_misconfigured` | Missing or invalid provider API key | Fix the provider key |
| `provider_attestation_required` | Provider needs an account confirmation (for example OpenRouter age verification) | Complete it in the provider settings |
| `model_unavailable` / `model_not_configured` | Model missing, or none set for the agent or session | Pick a configured model |
| `request_too_large` | Context overflowed even after compaction | Trim the conversation, start a fresh session |
| `invalid_tool_schema` | A tool definition was rejected by the provider | Fix the tool's schema |
| `budget_exhausted` / `budget_paused` | The budget has no room left, or is paused | Raise or resume the budget, see [Enforce a budget](/how-to/enforce-a-budget/) |
| `blocked_by_hook` | A `user_prompt_submit` hook rejected the message | Change the message or the hook |
| `processing_error` | Unclassified failure, or details hidden by disclosure mode | Check server logs |

Cancellation is not a failure: `sessions.cancel` produces `turn.cancelled`, which needs no retry.

## See also

- [Stream events](/how-to/stream-events/)
- [Event Reference](/event-reference/), all event types and payloads.
- [The agentic loop](/explanation/agentic-loop/#what-happens-when-the-loop-gets-stuck), failure modes.
