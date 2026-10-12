---
title: "Soft approval: agents pause before destructive or outward actions"
description: The new soft_approval capability, on by default for Generic and Platform Chat, has agents ask before risky actions and records who approved in the audit log.
date: 2026-09-17
kind: feature
significance: notable
products: [platform, cloud, framework]
surfaces: [agent]
release: "0.29.0"
links:
  docs: [/capabilities/soft-approval/]
  prs: [everruns/everruns#3655]
---

The `soft_approval` capability tells an agent to batch safe work and pause before
destructive or outward-facing actions such as deleting data, pushing, publishing
or sending messages. The pause is a `request_approval` tool call, so a waiting
turn is something a host can render. `record_approval` writes the grant and
`set_approval_mode` changes the level.

It is enabled at `normal` on the Generic and Platform Chat harnesses. The approver
is resolved on the server from the authenticated message, never from what the
model says, and recorded as `agent.approval.requested` and
`agent.approval.granted` in the audit log.

See [Soft approval](/capabilities/soft-approval/). [#3655](https://github.com/everruns/everruns/pull/3655)
