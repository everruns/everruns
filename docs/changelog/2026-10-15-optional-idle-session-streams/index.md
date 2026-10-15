---
title: React session streams can suspend while idle
description: Opt in to closing settled session event streams in the React UI client, with durable replay when sending, answering approvals, or returning to the tab.
date: 2026-10-15
kind: feature
significance: minor
products: [platform]
surfaces: [code]
links:
  docs: [/features/events/]
---

React UI embeddings can now close settled session event streams with
`followIdle: false`:

```tsx
const { data: events, resume } = useEvents(sessionId, { followIdle: false });
```

Message sends, approval answers, and tool results reopen the stream from its
durable cursor. Returning to the tab, reconnecting the network, or calling
`resume()` catches up events received while suspended. Active turns, parked
tools, pending sign-ins, and unfinished background tasks keep following.

Continuous following remains the default for schedules, channels, and external
updates. Idle suspension affects this event hook only; independent subscriptions
remain open. See [Events](/features/events/) for custom mutation handling and the
external-update tradeoff. Resource savings have not been measured.

## Update 2026-10-15

Optimistic sends now reconcile with stored inputs before rendering, preventing a
transient duplicate message or turn-status row when an event arrives before its
request acknowledgement.
