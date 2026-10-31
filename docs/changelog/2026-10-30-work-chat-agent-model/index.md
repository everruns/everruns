---
title: Work Chat uses the Agent's model
description: Work Chat no longer lists models people can pick. Messages use the model configured on the Agent, and a request that names another model is refused.
date: 2026-10-30
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/channels/#work-chat]
---

The Work Chat channel page no longer includes **Models consumers may select**.
The composer does not offer a model list. Every message uses the model
configured on the Agent. A channel that previously stored a list ignores it,
and the next save of that channel drops the list.

A message that names a model is refused. Reasoning effort, approvals, and
spending limits stay on the channel. Change the model on the Agent.

See [Work Chat](/features/channels/#work-chat).
