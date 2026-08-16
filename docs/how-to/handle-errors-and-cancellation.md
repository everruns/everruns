---
title: Handle errors and cancel turns
description: Detect and recover from failed turns, cancel a long-running turn, and react to common error events from the SSE stream.
---

Turns can fail (the LLM rejected the request, a tool errored repeatedly) or be cancelled by the user. The event stream tells you which. This guide covers both paths.

## Detect a failed turn

Failures surface as `turn.failed` events:

```python
async for event in client.events.stream(session.id):
    if event.type == "turn.completed":
        break
    if event.type == "turn.failed":
        err = event.data.get("error", "unknown")
        print(f"Turn failed: {err}")
        break
```

The `error` field is a structured object, type, message, optional cause. Inspect it to decide whether to retry, surface to the user, or escalate.

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

## Common error patterns

| Event payload | Cause | Action |
|---|---|---|
| `rate_limit_exceeded` | LLM provider rate-limited the worker | Wait, retry with backoff |
| `request_too_large` | Context overflowed even after compaction | Trim the conversation, start a fresh session |
| `tool_call_failed` (terminal) | A tool errored repeatedly | Inspect tool result events, fix the agent prompt or tool config |
| `cancelled` | User or app called `sessions.cancel` | No retry, user intent |

## See also

- [Stream events](/how-to/stream-events/)
- [Event Reference](/event-reference/), all event types and payloads.
- [The agentic loop](/explanation/agentic-loop/#what-happens-when-the-loop-gets-stuck), failure modes.
