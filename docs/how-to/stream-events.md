---
title: Stream events
description: Consume a session's SSE event stream from the Python SDK or any HTTP client, with reconnection via since_id, heartbeats, and event filtering.
appliesTo: [platform, cloud]
---

Every session publishes its events as a Server-Sent Events (SSE) stream. Use
the [Python SDK](#with-the-python-sdk) when you can: it handles reconnection,
heartbeat-based stale detection, backoff, and resumption with `since_id`. Use
the [raw SSE protocol](#with-raw-sse) from any other client, such as a
non-Python service, a browser, or `curl`.

## With the Python SDK

`client.events.stream(session_id)` returns an async iterator over typed events.

### Basic stream

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

### Tool visibility

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

### Get the full final message

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

### What the SDK handles for you

- **Reconnection.** The control plane cycles SSE connections every 5 minutes; the SDK reconnects transparently using `since_id`.
- **Stale detection.** The server sends a heartbeat every 30s; the SDK treats >45s of silence as a dead connection and reconnects.
- **Backoff.** Network errors trigger exponential backoff with jitter.
- **Typing.** Each event has `.type` and `.data` attributes parsed from SSE.

### Resuming with `since_id`

While a stream is open the SDK manages reconnection internally. You only need `since_id` when restarting your application and resuming from a previously recorded event ID:

```python
# Persisted somewhere — file, DB, etc.
last_seen_id = load_cursor()

async for event in client.events.stream(session.id, since_id=last_seen_id):
    handle(event)
    save_cursor(event.id)  # so the next restart can resume from here
```

Inside the loop the SDK already remembers the last ID it yielded and reconnects with it on transient failures, `save_cursor` here is for *application restart* recovery, not per-iteration SDK state.

## With raw SSE

### Subscribe

```bash
curl -N "https://your-host/api/v1/sessions/$SESSION_ID/sse" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY"
```

Each event arrives as:

```
event: turn.completed
id: event_01933b5a00007000800000000000001
data: {"id":"event_...","type":"turn.completed","data":{...}}

```

(Blank line terminates each event, per the SSE spec.)

### Resume after disconnect

Pass `since_id` to pick up where you left off:

```bash
curl -N "https://your-host/api/v1/sessions/$SESSION_ID/sse?since_id=event_..." \
  -H "Authorization: Bearer $EVERRUNS_API_KEY"
```

Event IDs are UUIDv7 and the server orders them by an atomic per-session sequence number. Resumption is gap-free and duplicate-free.

### Heartbeats

The server sends a heartbeat every 30 seconds as a comment line:

```
: heartbeat
```

Comments are invisible to SSE event parsers, they don't appear as events. Their only purpose is to keep the TCP connection alive and let your client distinguish "idle" from "dead."

**Client requirement:** treat the connection as stale if no data (event or heartbeat) arrives within 45 seconds. Reconnect with the last received event ID.

### Connection cycling

To avoid stale connections through proxies, the server gracefully cycles SSE connections every 5 minutes. Before closing, it sends:

```
event: disconnecting
data: {"reason":"connection_cycle","retry_ms":100}
```

Clients should reconnect immediately using `since_id` of the last event received. No events are dropped during the transition.

### Browser EventSource

```javascript
function connect(sessionId, lastEventId) {
  const url = new URL(`/api/v1/sessions/${sessionId}/sse`, API_BASE);
  if (lastEventId) url.searchParams.set("since_id", lastEventId);

  const es = new EventSource(url, { withCredentials: true });

  es.addEventListener("connected", () => console.log("SSE connected"));

  es.addEventListener("disconnecting", (e) => {
    const { retry_ms } = JSON.parse(e.data);
    es.close();
    setTimeout(() => connect(sessionId, lastEventId), retry_ms);
  });

  ["input.message", "output.message.delta", "turn.completed"].forEach((t) => {
    es.addEventListener(t, (e) => {
      const data = JSON.parse(e.data);
      lastEventId = data.id;
      // handle event...
    });
  });

  es.onerror = () => {
    es.close();
    setTimeout(() => connect(sessionId, lastEventId), 2000);
  };
}
```

The native `EventSource` API uses the `retry:` field that every event includes (100ms during active streaming, up to 500ms while idle). You don't need to set retry yourself.

### Poll as a fallback

If your environment can't hold long-lived connections (some serverless runtimes), poll instead:

```bash
curl "https://your-host/api/v1/sessions/$SESSION_ID/events?since_id=$LAST_ID" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY"
```

The same `since_id` resumption works; latency increases by the polling interval.

## See also

- [Event Reference](/event-reference/): every event type and payload.
- [Events as the primary store](/explanation/events/): why the protocol is shaped this way.
