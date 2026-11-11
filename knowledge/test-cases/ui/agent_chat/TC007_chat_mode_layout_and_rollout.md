---
type: Test Case
title: "TC007: Chat first and Classic layouts"
description: "Verify the layout preference, landing on Overview, the splitter, the navigation rail, full screen, browser back and forward, keyboard shortcuts, persistent Chat/page state and pane-scoped Agent sheets."
tags: [ui, test-case, agents, platform-chat]
---

# TC007: Chat first and Classic layouts

## Preconditions

- Any test organization; no feature needs enabling.
- Editable Agent; desktop and narrow viewports; light and dark themes.

## Steps

1. Sign in with the default layout and open `/`. The app lands on Overview with Main chat
   on the left and the navigation rail on the right.
2. Hover the divider between chat and page: a thin line appears. Drag it, double-click it
   and use the arrow keys after focusing it. The chat stays at least 360 px and the page at
   least 480 px. Reload: the width is kept. Drag the chat to the right edge: `/chats` opens.
3. In the rail, hover and keyboard-focus icons: tooltips name each page. Overview shows its
   count. Open **System and settings**: system pages and Settings are listed. Expand the
   rail with »: the full sidebar appears without the Chat section; « collapses it and the
   choice survives a reload. Hidden or restricted pages appear in neither form.
4. Open the profile avatar at the bottom of the rail: profile, organization, theme, layout
   and sign out are there.
5. Open an Agent and type a Chat composer draft. Press Command/Ctrl-Shift-J (or
   **Full screen**): the conversation goes full screen at `/chats` with the draft intact.
   Press it again (or **Show beside page**): the Agent returns and the draft is still there.
   Command/Ctrl-J focuses the composer without leaving the page.
6. Switch to a side chat with **Main chat ▾**: the page stays. Open a thread from the chat:
   the page pane shows `/threads/{id}`. Browser Back returns to the Agent; Forward reopens
   the thread.
7. With an Agent sheet (Network access or Branding) open, confirm the sheet and its backdrop
   stay inside the page pane and Chat remains interactive.
8. Switch the profile menu's layout to **Classic**. The left sidebar returns with Main chat
   and side chats; the rail and splitter are gone. Command/Ctrl-J toggles full-screen chat.
   Reload `/` in Classic: it lands on full-screen Main chat. The setting is the same on
   Profile → Appearance and after signing in on another device.
9. At 390 px, both layouts show one pane at a time with the mobile navigation drawer.
10. Repeat key steps in dark mode.

## Expected result

- The layout follows the person, not the organization; Chat first is the default.
- No width transitions; nothing remounts when moving between layouts.
- Every change is a browser history step except switching chats beside the page.
- Drawers affect only the page pane; hidden navigation does not retain keyboard focus.
