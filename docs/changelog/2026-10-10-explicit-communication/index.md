---
title: "Explicit communication lets agents choose when to reply"
description: "An agent's Communication setting can be Explicit, so assistant text stays as private notes and the agent speaks only through send_message or stays silent."
date: 2026-10-10
kind: feature
significance: notable
products: [platform, cloud, framework, cli]
surfaces: [ui, api, code, agent]
links:
  docs: [/features/explicit-communication/]
  prs: [everruns/everruns#4447]
---

Every agent now has a **Communication** setting. **Direct**, the default, keeps
today's behavior: the assistant text is the reply. **Explicit** treats
assistant text as private working notes; the agent talks only by calling
`send_message` and can call `no_reply` to stay silent.

This fits Slack channels and conversations with several people, where the
agent should answer only when it has something to add. The setting is on the
agent in the web UI, the API, agent packages, the Framework builder and the CLI,
and it replaces Slack's per-endpoint reply mode. Every sent message is recorded
as a `conversation.message` event.

See [Explicit communication](/features/explicit-communication/) and [#4447](https://github.com/everruns/everruns/pull/4447).
