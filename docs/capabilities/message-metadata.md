---
title: Message Metadata
description: Annotate user and agent messages with metadata such as their timestamp when they are sent to the LLM, so agents can reason about timing and gaps between messages.
---

| | |
|---|---|
| **ID** | `message_metadata` |
| **Category** | Core |
| **Features** | None |
| **Dependencies** | None |

Annotates user and agent messages with metadata, currently each message's timestamp (UTC), when building the LLM request. The model sees each message prefixed with an annotation like:

```
[time 2026-06-11T09:15:42Z] What changed since yesterday?
```

For user messages the timestamp is when the message was received; for agent messages, when the reply was generated.

This lets agents reason about timing: how long ago something was said, gaps between messages, and whether earlier statements are stale.

Enabled by default on the Generic (default) harness.

Annotations are applied only to the prompt-facing view of the conversation. Stored messages are never modified, and timestamps are stable across turns so prompt caching is unaffected. A short system prompt addition explains the annotation format to the model and instructs it not to emit annotations in its replies.

## Tools

None.

## Configuration

| Field | Type | Default | Description |
|---|---|---|---|
| `fields` | array | `["timestamp"]` | Metadata fields to render, in order. Supported: `timestamp`. An empty array disables annotations. More fields (e.g. the LLM model) will be added over time. |

User and agent messages are always annotated; system and tool-result messages never are.

## See Also

- [Current Time](/capabilities/current-time/), tool to get the current wall-clock time
- [Capabilities Overview](/capabilities/)
