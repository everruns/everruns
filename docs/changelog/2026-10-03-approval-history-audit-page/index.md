---
title: Approval history in chat and a new Approvals audit page
description: Chats show approval requests and recorded consent as clear entries, and organization admins get an Approvals page that filters approvals across sessions.
date: 2026-10-03
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui]
release: "0.37.0"
visual: "Approvals page under Operational with the Recorded filter selected and one consent entry linking to its session; crop to the table."
links:
  docs: [/capabilities/soft-approval/]
  prs: [everruns/everruns#4110]
---

Approval activity no longer looks like ordinary tool calls. In a chat,
`request_approval` and `record_approval` show as approval entries with the
action, the approver's name, and a link to the message that gave consent.

Organization admins and owners also get an **Approvals** page under
**Operational**. It filters all activity, recorded approvals, or requests, and
links each entry back to its session. It reads the existing approval audit
records, so earlier history appears too.

See [Soft approval](/capabilities/soft-approval/). PR [#4110](https://github.com/everruns/everruns/pull/4110).
