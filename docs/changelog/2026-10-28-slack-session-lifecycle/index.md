---
title: "Slack session status and controls follow the current API"
description: "Slack sessions now show processing, available and waiting states correctly, and stop and rename controls accept the current event format."
date: 2026-10-28
kind: fix
significance: minor
products: [platform]
surfaces: [agent]
links:
  docs: [/integrations/slack/]
---

[Slack agent sessions](/integrations/slack/) now send the lifecycle states expected by Slack's current
API. A running turn shows as processing, a parked tool call or approval ask waits
for input, and ordinary turn completion makes the session available again.
Finishing a reply does not close the session.

Stop and rename controls accept both current and legacy event formats. Each
control is checked against the session's recorded Slack conversation, including
when an integration is restricted to a channel. Conflicting channel or thread
identifiers are ignored. No configuration change is required for existing agent
pane sessions.
