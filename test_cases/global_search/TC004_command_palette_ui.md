# TC004: Command Palette - UI Navigation

## Description

Verify the Cmd+K / Ctrl+K command palette opens, shows navigation pages by default, searches entities, and navigates on selection.

## Preconditions

- UI running (dev or full mode)
- At least 1 agent and 1 session exist

## Steps

1. Press `Cmd+K` (macOS) or `Ctrl+K` (Linux/Windows)
2. Verify palette opens with default navigation pages (Dashboard, Sessions, Agents, etc.)
3. Type "agent" — verify agents appear in results alongside "Agents" navigation page
4. Use Arrow Down/Up to navigate results
5. Press Enter on a result — verify navigation to correct page
6. Open palette again, press Escape — verify palette closes
7. Type a long poem — verify no hang, shows "No results" message
8. Type an entity ID prefix (e.g. `agent_`) — verify "Go to" section appears

## Expected Results

- Steps 1-2: Palette opens with 6 default navigation pages
- Step 3: Results grouped by category (Pages, Agents)
- Step 4: Selection highlight moves with arrow keys
- Step 5: Palette closes and browser navigates to selected item's URL
- Step 6: Palette closes on Escape
- Step 7: "No results" displayed promptly (no lag)
- Step 8: ID-based lookup result with "Go to Agent" label
