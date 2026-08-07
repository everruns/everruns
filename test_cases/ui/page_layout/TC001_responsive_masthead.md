# Responsive page masthead

## Description

Verify that shared page mastheads preserve readable identity content and contain large action sets
when the app content area narrows.

## Preconditions

- Everruns is running in development mode with authentication disabled.
- An active agent exists with a title, description, metadata, and all optional detail-page actions.

## Test Data

| Field | Value |
|---|---|
| Route | Active agent detail page |
| Wide desktop viewport | 1440 × 1000 |
| Compact desktop viewport | 1024 × 900 |
| Tablet viewport | 768 × 900 |
| Mobile viewport | 390 × 844 |

## Steps

1. Open the agent detail page at the wide desktop viewport.
2. Confirm the action cluster is right-aligned beside the title, description, badges, and metadata.
3. Resize to the compact desktop viewport and confirm the action cluster moves below the identity
   content instead of squeezing it.
4. Resize to the tablet and mobile viewports and confirm actions wrap within the masthead.
5. Repeat with a long title, a long description, expanded button labels, badges, and metadata.
6. Inspect representative detail pages for agents, harnesses, apps, capabilities, skills, memory,
   knowledge indexes, agent identities, and sessions.

## Expected Result

- Identity content keeps a readable width instead of collapsing into a single-word column.
- Actions remain visible, usable, and contained by the masthead at every viewport.
- Long text wraps without horizontal page overflow.
- Wide desktop mastheads retain right-aligned actions and the established visual hierarchy.
- All consumers of the shared masthead follow the same responsive behavior without page-local
  width overrides.
