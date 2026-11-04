---
title: Claude Message Threads
description: Send only new messages on compatible Claude turns, with full local history and automatic replay when temporary thread state expires.
appliesTo: [framework, platform, cloud]
---

| | |
|---|---|
| **ID** | `claude_message_threads` |
| **Category** | Optimization |
| **Tools** | None |
| **Features** | None |
| **Dependencies** | None |

Enable this capability on an agent or harness to use Anthropic's optional
message threads beta. The first request sends the full conversation; compatible
later turns send only new messages. It adds no tools or system prompt text.
Other provider drivers ignore the option.

Everruns keeps the full transcript. If a thread expires or cannot be continued,
the driver replays that history. Restarts, edits, changed settings, credentials,
or concurrent turns can start fresh threads. Compaction and batch requests stay
unthreaded; incomplete responses never advance thread state.

**Data retention:** Anthropic temporarily retains thread content. This beta is
not eligible for zero data retention or HIPAA. Enable it only when that retention
is acceptable for your workload.

Threads reduce repeated history uploads, but do not change token pricing.
[Prompt Caching](/capabilities/prompt-caching/) remains a separate capability.
See [the Anthropic provider guide](/providers/anthropic/#message-threads-beta)
for configuration examples, retention details, and recovery behavior.
