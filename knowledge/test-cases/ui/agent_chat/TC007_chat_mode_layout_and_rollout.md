---
type: Test Case
title: "TC007: Chat beside the page and full screen"
description: "Verify the sidebar entry opens Chat beside the page, full screen in both directions, keyboard shortcuts, persistent Chat/page state, accessible navigation and pane-scoped Agent sheets."
tags: [ui, test-case, agents, platform-chat]
---

# TC007: Chat beside the page and full screen

## Preconditions

- Any test organization; no feature needs enabling.
- Editable Agent; desktop and narrow viewports; light/dark and reduced-motion emulation.

## Steps

1. Open an Agent and click **Main chat** in the sidebar. Chat opens beside the Agent page;
   the URL stays on the Agent. Modifier-click instead opens `/chats` as a link.
2. Start an unsaved form edit and type a Chat composer draft. Toggle Chat with
   Command/Ctrl-J and **Full page**. Confirm both drafts and the page route survive.
3. Press Command/Ctrl-Shift-J (or **Full screen**): the conversation goes full screen at
   `/chats`. Press it again (or **Show beside page**): the Agent returns with Chat beside it
   and the unsaved form edit intact.
4. Click a side chat in the sidebar while on the Agent page: it opens beside the page.
   **Main chat ▾** lists Main chat, the open chat's threads and side chats.
5. While Chat is open, open Network access or Branding. Confirm the 440 px sheet
   and its backdrop stay inside the page pane and Chat remains interactive.
6. Use keyboard navigation. The collapsed ordinary sidebar must be inert; the
   active rail and visible Chat/page controls remain reachable, without duplicate
   hidden navigation stops. Closing Chat returns focus to the page.
7. At 390 px, wait for the transition to settle. Confirm Chat and Full page
   fit the viewport, the page returns without remounting, and the composer is usable.
8. Repeat key steps in dark mode and with reduced motion. Reduced motion removes
   the grid transition; colors, focus and change markers remain readable.
9. Open `/chats` directly and via Command-K; confirm one full conversation with no
   page-context chip. Command/Ctrl-J focuses its composer. `/chats/history` stays a list.
10. Hover and keyboard-focus each rail icon: its tooltip must show a page name and short
    explanation, stay within the viewport and dismiss with Escape.
11. Open Pages by click, keyboard and touch. Verify a grouped, scrollable menu of
    names and descriptions; the current page is identified. Escape returns focus
    to Pages. Disabled flags and role/policy restrictions hide destinations in both
    the rail and this menu. Closing Chat removes every open rail overlay.

## Expected result

- The same layouts for every organization; no flag gates them.
- Chat, page state and composer survive every layout change; organization changes reset them.
- Drawers affect only the page pane; hidden navigation does not retain keyboard focus.
- Settled desktop/mobile layouts fit, and reduced motion and both themes work.

After a real Chat patch, confirm the draft notice stays inside the workspace,
the document does not grow beyond the viewport, and the Chat header remains
visible. Scroll the Agent pane to its field Undo and back; Chat must retain its
own scroll and composer.
