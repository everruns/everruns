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

## Resuming with `since_id`

While a stream is open the SDK manages reconnection internally. You only need `since_id` when restarting your application and resuming from a previously recorded event ID:

```python
# Persisted somewhere — file, DB, etc.
last_seen_id = load_cursor()

async for event in client.events.stream(session.id, since_id=last_seen_id):
    handle(event)
    save_cursor(event.id)  # so the next restart can resume from here
```

Inside the loop the SDK already remembers the last ID it yielded and reconnects with it on transient failures, `save_cursor` here is for *application restart* recovery, not per-iteration SDK state.

## See also

- [Event Reference](/event-reference/), all event types.
- [Events as the primary store](/explanation/events/), why the protocol is shaped this way.
- [Consume events via raw SSE](/how-to/consume-events-via-sse/), non-SDK clients.
