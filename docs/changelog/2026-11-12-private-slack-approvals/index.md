---
title: Private approval cards for Slack requesters
description: Slack channels can show approval cards only to the requester, keeping permission checks intact and offering session-based recovery when a card disappears.
date: 2026-11-12
kind: feature
significance: minor
products: [platform]
surfaces: [ui, api]
links:
  docs: [/capabilities/slack/#approvals]
---

Enable **Private approval cards** in a Slack channel’s conversation settings to
show its approval cards only to the person who made the request. Existing channels
keep their current thread-visible behavior. The same option is available in
`channel_config`:

```json
{
  "private_approvals": true
}
```

[Private approvals](/capabilities/slack/#approvals) keep requester and single-use
checks. Failed delivery never posts the private question publicly. Slack ephemeral
cards can disappear; answer the pending request in Everruns if that happens.
Agent replies still follow the conversation’s normal visibility.
