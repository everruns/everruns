---
title: Chat restores drafts and uncertain message sends
description: Chat keeps unsent text and pending messages through a tab reload, preserves safe retry identities, and shares live conversation subscriptions.
date: 2026-10-28
kind: improvement
significance: notable
products: [platform]
release: "0.50.0"
surfaces: [ui, api]
links:
  docs: [/built-ins/harnesses/platform-chat/, /features/agent-api/]
---

Chat restores unsent text and pending messages in the same tab. **Retry**
preserves the send; **Discard** leaves accepted work running. Storage failures
keep text editable.

[Platform Chat](/built-ins/harnesses/platform-chat/) shares live session
subscriptions with task views and reconnects after network return. Hosted
Work Chat also preserves uncertain sends across reloads and stops stream
retries when access is rejected. Its retries use the
[Agent API's idempotency contract](/features/agent-api/).

Recovery is scoped to the login and conversation. Logout clears it. Closing
the tab or clearing site data can remove recovery. This provides no offline
model or application shell.

![Hosted Work Chat offers Retry and Discard for a recovered send](./recovery@2x.png)

## Update 2026-10-31

These fixes follow 0.50.0.

Hosted Work Chat streams stay open past 30 seconds. Connection timeouts cover
setup; leaving the conversation closes the stream.

Worker recovery preserves the saved answer and work counters. New keyed
admissions commit with their replay response, so retries recover after server
restarts.
