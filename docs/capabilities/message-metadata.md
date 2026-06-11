---
title: Message Metadata
description: Annotate user and agent messages with metadata such as sent time when they are sent to the LLM, so agents can reason about timing and gaps between messages.
---

| | |
|---|---|
| **ID** | `message_metadata` |
| **Category** | Utilities |
| **Features** | None |
| **Dependencies** | None |

Annotates user and agent messages with metadata — currently the time each message was sent (UTC) — when building the LLM request. The model sees each message prefixed with an annotation like:

```
[sent 2026-06-11T09:15:42Z] What changed since yesterday?
```

This lets agents reason about timing: how long ago something was said, gaps between messages, and whether earlier statements are stale.

Annotations are applied only to the prompt-facing view of the conversation. Stored messages are never modified, and timestamps are stable across turns so prompt caching is unaffected. A short system prompt addition explains the annotation format to the model and instructs it not to emit annotations in its replies.

## Tools

None.

## Configuration

| Field | Type | Default | Description |
|---|---|---|---|
| `user_messages` | boolean | `true` | Annotate user messages with sent time |
| `agent_messages` | boolean | `true` | Annotate agent messages with sent time |

## See Also

- [Current Time](/capabilities/current-time/) — tool to get the current wall-clock time
- [Capabilities Overview](/capabilities/)
