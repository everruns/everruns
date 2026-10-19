---
title: Schedule triggers can run in a Slack conversation
description: A schedule trigger can now run in a direct message or channel of the Agent's own Slack channel, sharing that conversation's session from the first run.
date: 2026-10-18
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [api, cli, agent]
links:
  docs: [/features/agent-triggers/]
---

A schedule trigger used to run in a session of its own, with no way to reach
the people who talk to the Agent in Slack. An Agent that briefs someone every
morning had to schedule itself from inside the conversation, so nothing ran
until that person had written first.

A schedule trigger now takes a Slack target: one of the Agent's own Slack
channels and a conversation on it, `slack:user:{user_id}` for a direct message
or `slack:channel:{channel_id}` for top-level posts. Each run lands in the
session a message from that conversation would, and its replies post there.
The first run opens the direct message, even before the person has written.
Only someone who can manage the Agent can set a target. A channel that can no
longer post fails the run visibly instead of falling back to a session of its
own. See
[Run in a Slack conversation](/features/agent-triggers/#run-in-a-slack-conversation).

```json
{
  "trigger_type": "schedule",
  "cron_expression": "0 7 * * *",
  "timezone": "America/Chicago",
  "message": "Morning briefing",
  "target_channel_id": "appchan_01933b5a000070008000000000000001",
  "target_binding_key": "slack:user:U0123ABCD"
}
```
