---
title: Slack agents set to Mentions only answer their @-mentions again
description: A Slack channel set to answer mentions only, or relevant messages, no longer drops an @-mention in a channel, and editing its settings keeps its workspace.
date: 2026-11-06
kind: fix
significance: minor
products: [platform]
surfaces: [ui, api, cli]
---

Slack sends one @-mention in a channel as two events with the same timestamp:
a plain message and a mention. When the plain message arrived first, a channel
with the **Mentions only** response policy declined it and then took the
mention for a duplicate, so the agent stayed silent: no session, no reply, no
health issue. **Relevant messages** could lose a mention the same way.

The two events no longer cancel each other. Only a message the agent is going
to answer counts toward duplicate detection, and a plain message that
@-mentions the agent's bot user is treated as a mention. Each @-mention now
starts exactly one turn under every response policy, whichever event Slack
delivers first, and a real Slack retry is still dropped.

Updating a Slack channel's settings with a full config, such as
`everruns agents channels update --channel-config`, also keeps the channel's
Slack workspace (`team_id`) when the new config leaves it out. The workspace
scopes which events the channel accepts and addresses its replies.
