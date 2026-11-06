---
title: "Run a trigger on an event you choose"
description: "Fire any agent trigger with an event, or point a GitHub trigger at an open pull request, and it runs exactly as a real delivery would."
date: 2026-11-03
release: "0.51.0"
kind: feature
significance: notable
products: [platform, cloud, cli]
surfaces: [api, cli, agent]
links:
  docs: [/features/agent-triggers/]
---

Testing a GitHub, webhook or MCP event trigger used to mean opening a real pull
request or posting to the webhook yourself. Now
[fire the trigger with an event](/features/agent-triggers/#fire-a-trigger-with-an-event)
and it takes the trigger's real path: same filter, same message, same
per-pull-request session, and a delivery marked `manual` in its history.

For a GitHub trigger, name the pull request. Everruns reads it through the
agent's own GitHub connection and builds the event GitHub would have sent, so a
review agent picks up where the last run on that pull request left off:

```bash
everruns agents triggers trigger --agent-id agent_01h9 --trigger-id trg_01h9 \
  --event '{"repository":"acme/api","pull_request":12}' --reason 'Review this pull request'
```

```json
{
  "status": "filtered",
  "session_id": null,
  "created_session": false,
  "reason": "payload.pull_request.draft is true"
}
```

A filtered event says which condition it missed, as above for a draft.
