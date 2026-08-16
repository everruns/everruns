---
title: Consume events via raw SSE
description: Subscribe to the Everruns event stream from any HTTP client using Server-Sent Events, with reconnection via since_id.
---

When you can't use the SDK, a non-Python service, a browser client, a Postman test, the SSE protocol is available directly. This guide covers the protocol details you need.

For the SDK convenience layer, see [Stream events](/how-to/stream-events/).

## Subscribe

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

## Resume after disconnect

Pass `since_id` to pick up where you left off:

```bash
curl -N "https://your-host/api/v1/sessions/$SESSION_ID/sse?since_id=event_..." \
  -H "Authorization: Bearer $EVERRUNS_API_KEY"
```

Event IDs are UUIDv7 and the server orders them by an atomic per-session sequence number. Resumption is gap-free and duplicate-free.

## Heartbeats

The server sends a heartbeat every 30 seconds as a comment line:

```
: heartbeat
```

Comments are invisible to SSE event parsers, they don't appear as events. Their only purpose is to keep the TCP connection alive and let your client distinguish "idle" from "dead."

**Client requirement:** treat the connection as stale if no data (event or heartbeat) arrives within 45 seconds. Reconnect with the last received event ID.

## Connection cycling

To avoid stale connections through proxies, the server gracefully cycles SSE connections every 5 minutes. Before closing, it sends:

```
event: disconnecting
data: {"reason":"connection_cycle","retry_ms":100}
```

Clients should reconnect immediately using `since_id` of the last event received. No events are dropped during the transition.

## Browser EventSource

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

## Poll as a fallback

If your environment can't hold long-lived connections (some serverless runtimes), poll instead:

```bash
curl "https://your-host/api/v1/sessions/$SESSION_ID/events?since_id=$LAST_ID" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY"
```

The same `since_id` resumption works; latency increases by the polling interval.

## See also

- [Event Reference](/event-reference/), every event type and payload.
- [Events as the primary store](/explanation/events/), why the protocol is shaped this way.
- [Stream events with the SDK](/how-to/stream-events/), the convenient path.
