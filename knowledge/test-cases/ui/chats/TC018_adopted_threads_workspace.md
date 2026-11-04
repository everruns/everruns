---
type: Test Case
title: "TC018: Chat - Subchats and Adopted Background Work"
description: "Verify independent Chat instances, single-conversation layouts, and separately opted-in background work."
tags:
  - everruns
  - test-case
  - ui
  - chats
---
# TC018: Chat - Subchats and Adopted Background Work

## Preconditions

- Canonical DB-backed stack with two organizations and owner/admin plus member accounts.
- Personal side conversations, archived conversations, and background work exist.

## Steps

1. Test all four combinations of Chat workspace and Chat background work. Confirm Chat,
   New subchat, recent subchats and history remain available in each combination.
2. With no subchats, confirm the sidebar still exposes New subchat. Open it and
   confirm one empty draft replaces the conversation. No session exists before first send.
3. Send a message; confirm a distinct session and URL, the same managed Agent and
   execution harness, and no copied transcript or workspace files. Fail and retry the
   first send; confirm only one session is created.
4. Switch between permanent Chat and two subchats. Each selection replaces the main
   transcript; the Subchats panel contains a list, never another composer. Check history,
   search, pagination, pin/archive and reopening an archived personal conversation.
5. Enable Chat background work as owner/admin. Confirm a member cannot change enrollment.
   The Work tab is separate from the Subchats list. Disabling it preserves all conversations.
6. Start work from subchat A, then switch to B. Confirm each Work view belongs to
   its own Chat. Open a delegated child; its Work view belongs to its validated parent.
   Unknown task IDs must never fetch another conversation's work.
7. Inspect working, input-required, failed and succeeded work. Answer an input request,
   stop active work and resolve/reopen an idle delegated child. Idle alone is not resolved.
8. With Chat workspace enabled, navigate from a subchat to an editable Agent. Open
   page-side Chat with Command/Ctrl-J. Confirm the same personal instance, draft and
   attachments; inspect its page context. Open full Chat and confirm one conversation,
   no page context to Chat itself, and no draft loss. A delegated child must not become
   the page editor when returning to an app page.
9. Close/back from the Subchats or Work panel. Confirm the selected conversation stays
   selected; closing work detail removes its task selection rather than resetting Chat.
10. Verify wide, 390px, light/dark and reduced-motion layouts, drawer focus, and that
    hiding page-side Chat closes any panel portal. The panel must fit the actual Chat pane.
11. Send a page-draft request, then open full Chat before its tool result arrives.
    Confirm it reports that the original form is unavailable, changes no other page,
    and lets the conversation continue. Repeat by only hiding Chat while staying
    on the same form: the authorized request may finish on that original draft.
    A fresh full Chat must never acquire a page binding.
12. Switch organizations while Chat or a draft is open. Confirm no composer, task,
    page context or review state crosses organizations; disable enrollment and recheck.

## Expected Result

- Adoption is tenant opt-in; deployment availability never enrolls an organization.
- Permanent Chat remains protected. Idle does not imply resolved.
- Existing conversations, URLs, and task contracts are reused without copying history.
- Playground conversations remain outside Chat; task access and mutations retain server authorization.
- Responsive layouts do not overflow, and closing the mobile drawer restores Chat focus.
