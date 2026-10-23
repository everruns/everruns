---
type: Test Case
title: "TC007: Chat mode layout, persistence and rollout"
description: "Verify Adoption gating, persistent Chat/page state, accessible navigation and pane-scoped Agent sheets across layouts."
tags: [ui, test-case, agents, platform-chat, feature-flags]
---

# TC007: Chat mode layout, persistence and rollout

## Preconditions

- Test organization with Chat mode initially off; operator may manage Features.
- Editable Agent; desktop and narrow viewports; light/dark and reduced-motion emulation.

## Steps

1. With the org flag off, verify the ordinary sidebar, Agent and standalone Chat
   flows. Deployment availability alone must not enable the split workspace.
2. Enable Chat mode in Settings → Features. Open an Agent, start an unsaved form
   edit, and type a Chat composer draft. Toggle Chat using the sidebar and
   Command/Ctrl-J. Confirm both drafts and the page route survive.
3. While Chat is open, open Network access or Branding. Confirm the 440 px sheet
   and its backdrop stay inside the page pane and Chat remains interactive.
4. Use keyboard navigation. The collapsed ordinary sidebar must be inert; the
   active rail and visible Chat/page controls remain reachable, without duplicate
   hidden navigation stops. Closing Chat returns focus to the page.
5. At 390 px, wait for the transition to settle. Confirm Chat and Back to app
   fit the viewport, the page returns without remounting, and the composer is usable.
6. Repeat key steps in dark mode and with reduced motion. Reduced motion removes
   the grid transition; colors, focus and change markers remain readable.
7. Open Chat via Command-K and directly via `/chats`; confirm both enter the shared
   workspace. Verify side-chat URLs retain their existing behavior.
8. Turn the flag off and reload. Confirm the ordinary shell returns and browser
   draft tools are absent from the managed Agent's runtime tool definitions.

## Expected result

- Opt-in is organization-effective and independent of Chat threads.
- Chat, page state and composer survive open/close; organization changes reset them.
- Drawers affect only the page pane; hidden navigation does not retain keyboard focus.
- Settled desktop/mobile layouts fit, and reduced motion and both themes work.

After a real Chat patch, confirm the draft notice stays inside the workspace,
the document does not grow beyond the viewport, and the Chat header remains
visible. Scroll the Agent pane to its field Undo and back; Chat must retain its
own scroll and composer.
