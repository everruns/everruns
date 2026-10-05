---
type: Test Case
title: "TC001: Command Palette - UI Navigation"
description: "Verify the Cmd+K / Ctrl+K command palette opens, shows navigation pages by default, searches entities, and navigates on selection."
tags:
  - everruns
  - test-case
  - ui
  - global-search
---
# TC001: Command Palette - UI Navigation

## Description

Verify the Cmd+K / Ctrl+K command palette opens, shows navigation pages by default, searches entities, and navigates on selection.

## Preconditions

- UI running (dev or full mode)
- At least 1 agent and 1 session exist
- User belongs to at least 2 organisations

## Steps

1. Press `Cmd+K` (macOS) or `Ctrl+K` (Linux/Windows)
2. Verify palette opens with available navigation pages in the current layout groups, starting with Chat and side-chat shortcuts
3. Type "agent" — verify agents appear in results alongside "Agents" navigation page
4. Use Arrow Down/Up to navigate results
5. Press Enter on a result — verify navigation to correct page
6. Open palette again, press Escape — verify palette closes
7. Type a long poem — verify no hang, shows "No results" message
8. Type an entity ID prefix (e.g. `agent_`) — verify "Go to" section appears
9. Search for another organisation by name — verify it appears under "Organisations"
10. Select that organisation — verify the current organisation switches without going to settings
11. Search for Playground, Exposures, Environments, Slack workspaces, and Health; open each result and verify its destination.
12. Search "settings" and verify all available Settings destinations remain discoverable, including personal access tokens.
13. Disable optional modules and machine payments; verify they disappear from navigation results.
14. Test as an organization member: Approvals is hidden; denied Durable Execution pages are hidden. In production, Dev Tools is hidden.
15. Repeat opening, searching, arrow navigation, selection, and dismissal at 390px width and in dark mode.

## Expected Result

- Steps 1-2: Palette opens with all available destinations in layout order; collapsed sections are searchable
- Step 3: Pages use their layout groups; entity results use their category (Agents, Sessions, etc.)
- Step 4: Selection highlight moves with arrow keys
- Step 5: Palette closes and browser navigates to selected item's URL
- Step 6: Palette closes on Escape
- Step 7: "No results" displayed promptly (no lag)
- Step 8: ID-based lookup result with "Go to Agent" label
- Steps 9-10: Palette closes and the selected organisation becomes current

- Steps 11-12: Current page names and Settings destinations match the navigation models.
- Steps 13-14: Visibility matches sidebar feature, role, policy, and development gates.
- Step 15: The menu fits the viewport and remains readable and keyboard accessible.
