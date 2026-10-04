---
type: Test Case
title: "TC018: Chat - Adopted Threads Workspace"
description: "Verify organization opt-in, permanent Chat, separate thread context, and work lifecycle."
tags:
  - everruns
  - test-case
  - ui
  - chats
---
# TC018: Chat - Adopted Threads Workspace

## Preconditions

- Canonical DB-backed stack with two organizations and owner/admin plus member accounts.
- Personal side conversations, archived conversations, and background work exist.

## Steps

1. Confirm the Chat threads feature defaults off and existing sidebar links remain available.
2. Enable Chat threads in Features as an owner/admin; confirm a member cannot change enrollment.
3. Confirm the sidebar offers only Chat; open Threads from the permanent conversation.
4. Search and paginate conversations; open an existing thread and a New thread draft.
5. Confirm permanent Chat keeps its transcript and the draft creates no session before first send.
6. Send the draft's first message; confirm separate context, fixed Agent/Generic binding, and a stable URL.
7. Resolve an idle thread; confirm it moves into Resolved, preserves its transcript, and offers Reopen before continuing.
8. Inspect working, input-required, failed, and succeeded work. Answer an input request; stop active work.
9. Follow View thread from a work card. Verify unknown work IDs do not fetch another owner's work.
10. Verify panel expansion, close/back navigation, keyboard access, and 390px light/dark drawers.
11. Switch to the non-enrolled organization and disable enrollment; confirm the default UI returns and history remains intact.

## Expected Result

- Adoption is tenant opt-in; deployment availability never enrolls an organization.
- Permanent Chat remains protected. Idle does not imply resolved.
- Existing conversations, URLs, and task contracts are reused without copying history.
- Playground conversations remain outside Chat; task access and mutations retain server authorization.
- Responsive layouts do not overflow, and closing the mobile drawer restores Chat focus.
