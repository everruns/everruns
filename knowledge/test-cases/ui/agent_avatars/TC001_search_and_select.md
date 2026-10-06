---
type: Test Case
title: "TC001: Search and select an agent avatar"
description: "Verify curated search, persistence, both shapes, custom replacement and removal."
tags:
  - everruns
  - test-case
  - ui
  - agents
---
# TC001: Search and select an agent avatar

## Preconditions

- API, worker, UI and database running; local `AUTH_MODE=none` or authenticated agent manager.
- Editable custom agent exists.

## Steps

1. Open the agent, enter Edit mode and open Branding.
2. Choose preset. Confirm 25 image-only tiles across five families, with accessible names and descriptions.
3. Search `navy developer fox`. Select Patch and inspect its square and circular previews.
4. Use avatar. Reload the page and reopen the picker; Patch remains current.
5. Search `customer service`, `code review`, and a nonexistent term. Clear search and filter by Bloom.
6. Select Mint and save. Confirm both square and circular appearances retain the same identity.
7. Upload a custom PNG, crop and save. Reopen picker; no catalog card is current.
8. Remove avatar and reload. Avatar is absent.
9. Check keyboard navigation, small viewport, light and dark themes. Cancel never submits the agent draft.
10. Archive the test agent, reopen it and open Branding. Avatar mutation controls are disabled.

## Expected Result

Search ignores case and surrounding whitespace; all words can match different metadata fields.
Tiles and selected square/circle previews display no names, roles or descriptions; search still
matches that metadata and screen readers can identify each tile.
Empty queries show all avatars. No matches offers clear filters. Selection saves immediately and
survives reload. Upload, replacement and removal still work; save progress blocks duplicate
mutations, failures allow retry, and read-only views cannot mutate. Names and roles do not change
agent behavior. Keyboard focus remains in the picker and returns on close.
