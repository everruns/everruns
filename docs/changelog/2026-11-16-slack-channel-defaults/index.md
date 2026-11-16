---
title: New Slack channels join the room as a teammate
description: A new Slack channel now keeps one session per Slack channel and answers only relevant messages, so the agent follows the room instead of replying to every line.
date: 2026-11-16
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui, api, cli]
links:
  docs: [/how-to/publish-to-slack/, /capabilities/slack/]
---

Drop an agent into `#support` and it no longer answers "brb, coffee" or starts a fresh memory
in every thread. New Slack channels now default to **Per Channel** sessions and the
**Relevant messages** response policy: one conversation per Slack channel, and a reply only for
DMs, `@mentions`, and clear requests within the agent's purpose. See
[Publish an agent as a Slack app](/how-to/publish-to-slack/).

```json
{
  "session_strategy": "per_channel",
  "response_policy": "relevant_messages"
}
```

Existing channels keep exactly what they were saved with. Pick **Per Thread** or
**All messages** in the channel editor, or set them in the
[channel config](/capabilities/slack/), to get the old behavior on a new channel.
