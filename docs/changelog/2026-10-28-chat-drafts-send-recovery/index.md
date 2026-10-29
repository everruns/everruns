---
title: Chat restores drafts and uncertain message sends
description: Chat keeps unsent text and pending messages through a tab reload, preserves safe retry identities, and shares live conversation subscriptions.
date: 2026-10-28
kind: improvement
significance: notable
products: [platform]
surfaces: [ui]
links:
  docs: [/built-ins/harnesses/platform-chat/, /features/agent-api/]
---

Chat restores unsent text and unacknowledged messages when you return to a
conversation in the same browser tab. Restored messages offer **Retry** and
**Discard**. Retry preserves the original send; discarding a local copy does
not cancel work the server already accepted. Storage failures leave text
editable and explain how to preserve it.

[Platform Chat](/built-ins/harnesses/platform-chat/) shares live session
subscriptions with task views and reconnects after network return. Hosted
Work Chat also preserves uncertain sends across reloads and stops stream
retries when access is rejected. Its retries use the
[Agent API's idempotency contract](/features/agent-api/).

Recovery stays scoped to the account or browser login and conversation.
Logout clears it. Closing the tab or clearing site data can remove local
recovery; this does not provide an offline model or application shell.

![Hosted Work Chat offers Retry and Discard for a recovered send](./recovery@2x.png)

## Update 2026-10-29

Hosted Work Chat keeps healthy live streams open past 30 seconds. Its upstream
connection timeout now covers setup without interrupting an established stream;
leaving the conversation still closes the connection.
