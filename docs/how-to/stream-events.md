---
title: Stream events with the SDK
description: Consume the SSE event stream from the Python SDK with automatic reconnection, heartbeat detection, and event filtering.
---

The Python SDK's `client.events.stream(session_id)` returns an async iterator over typed events. It handles reconnection, heartbeat-based stale detection, and resumption with `since_id` automatically.

## Basic stream

```python
async for event in client.events.stream(session.id):
    if event.type == "output.message.delta":
        print(event.data.get("delta", ""), end="", flush=True)
    elif event.type == "turn.completed":
        print()
        break
    elif event.type == "turn.failed":
        print(f"\n[failed: {event.data.get('error')}]")
        break
```

## Tool visibility

To show what the agent is doing while it works, listen for `tool.started` and `tool.completed`:

```python
async for event in client.events.stream(session.id):
    if event.type == "tool.started":
        tool_call = event.data.get("tool_call", {})
        print(f"  [tool] {tool_call.get('name')}")
    elif event.type == "tool.completed":
        status = "ok" if event.data.get("success") else "error"
        print(f"  [tool] {event.data.get('tool_name')}: {status}")
    elif event.type == "turn.completed":
        break
```

## Get the full final message

`output.message.completed` carries the complete final message after streaming finishes:

```python
async for event in client.events.stream(session.id):
    if event.type == "output.message.completed":
        message = event.data.get("message", {})
        for part in message.get("content", []):
            if part.get("type") == "text":
                print(part["text"])
    elif event.type == "turn.completed":
        break
```

## What the SDK handles for you

- **Reconnection.** The control plane cycles SSE connections every 5 minutes; the SDK reconnects transparently using `since_id`.
- **Stale detection.** The server sends a heartbeat every 30s; the SDK treats >45s of silence as a dead connection and reconnects.
- **Backoff.** Network errors trigger exponential backoff with jitter.
- **Typing.** Each event has `.type` and `.data` attributes parsed from SSE.

## Filtering with `since_id`

To pick up where a previous stream left off:

```python
last_id = None
async for event in client.events.stream(session.id, since_id=last_id):
    last_id = event.id
    ...
```

Pass `since_id` on the *initial* call when resuming after an application restart. While the stream is open, the SDK manages `since_id` internally.

## See also

- [Event Reference](/event-reference/) — all event types.
- [Events as the primary store](/explanation/events/) — why the protocol is shaped this way.
- [Consume events via raw SSE](/how-to/consume-events-via-sse/) — non-SDK clients.
