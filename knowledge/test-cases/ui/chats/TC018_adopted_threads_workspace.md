---
type: Test Case
title: "TC018: Chat - Side Chats and Threads"
description: "Verify Main chat and side chats as coordinators, the Threads panel on the right, opening threads beside the chat or the page, and the chat switcher."
tags:
  - everruns
  - test-case
  - ui
  - chats
---
# TC018: Chat - Side Chats and Threads

## Preconditions

- Canonical DB-backed stack with two organizations and owner/admin plus member accounts.
- No Chat feature is enabled in Settings → Features (there is none to enable).
- Personal side chats, archived conversations, and background work exist.

## Steps

1. In an organization that never opted into anything, open `/chats`. Confirm Main chat,
   the **Side chats +** sidebar group, and **Threads** in the conversation header.
2. With no side chats, confirm the sidebar still exposes **New side chat**. Open it and
   confirm no session is created until first send.
3. Ask Main chat for longer work. Confirm it says it is starting a thread, the Threads panel
   opens by itself on the right, and the row sits under Working with its live status line.
   **Threads · N** counts unresolved threads.
4. Switch between Main chat and two side chats. Each selection replaces the transcript;
   each chat's Threads panel lists only the threads it started. Start a thread from side
   chat A and confirm it is absent from B and from Main chat.
5. Click a thread row. Confirm it opens in the right panel next to the chat, with
   "← Threads", expand-to-full-width and close. The chat stays usable on the left. Answer a
   thread that needs you and confirm the row moves out of Needs you.
6. Open an app page and press Command/Ctrl-J. Use **Main chat ▾** to switch to a side chat
   and to open one of its threads. The thread replaces the page in the page pane with a
   back button naming the page; going back restores the page with its unsaved edits.
7. Stop active work and resolve/reopen an idle thread. Idle alone is not resolved; Resolved
   rows collapse at the bottom of the panel and drop out of the count.
8. Close/back from the panel. Confirm the selected conversation stays selected; closing a
   work detail removes its task selection rather than resetting Chat.
9. Verify wide, 1279px (drawer), 390px, light/dark and reduced-motion layouts, drawer focus,
   and that hiding page-side Chat closes any panel portal.
10. Switch organizations while Chat or a thread is open. Confirm no composer, thread, page
    context or review state crosses organizations.

## Expected Result

- Every organization gets the same model; no flag enrolls or hides it.
- Main chat remains protected. Idle does not imply resolved.
- Threads belong to the chat that started them; existing URLs and task contracts are reused.
- Playground conversations remain outside Chat; task access and mutations retain server authorization.
- Responsive layouts do not overflow, and closing the drawer restores focus to Threads.
