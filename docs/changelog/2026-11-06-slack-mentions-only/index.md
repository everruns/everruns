---
title: Slack agents set to Mentions only answer their @-mentions again
description: A Slack channel set to answer mentions only, or relevant messages, no longer drops an @-mention in a channel, and editing its settings keeps its workspace.
date: 2026-11-06
release: "0.51.0"
kind: fix
significance: minor
products: [platform]
surfaces: [ui, api, cli]
---

Slack sends one @-mention in a channel as two events with the same timestamp:
a plain message and a mention. When the plain message arrived first, a channel
set to **Mentions only** declined it and took the mention for a duplicate, so
the agent stayed silent. **Relevant messages** could lose mentions the same way.

Now only a message the agent will answer counts toward duplicate detection, and
a plain message that @-mentions the agent's bot user counts as a mention. Each
@-mention starts exactly one turn, whichever event arrives first.

Updating a Slack channel with a full config, such as
`everruns agents channels update --channel-config`, also keeps its workspace
(`team_id`) when the new config leaves it out.
