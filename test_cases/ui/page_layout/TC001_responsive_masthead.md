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
3. Resize to the compact desktop and tablet viewports. Confirm `New session` and `More actions`
   remain prioritized while Edit, Create app, Copy, and Export move to a second row. Open the
   overflow menu and confirm Observe this agent is available.
4. Resize to the mobile viewport. Confirm only `New session` and `More actions` remain visible.
   Open the overflow menu and confirm Copy, Export, Edit, Create app, and Observe this agent are
   available. Open the mobile navigation drawer and close it with Escape.
5. Repeat with a long title, a long description, expanded button labels, badges, and metadata.
6. Inspect representative detail pages for agents, harnesses, apps, capabilities, skills, memory,
   knowledge indexes, agent identities, and sessions.

## Expected Result

- Identity content keeps a readable width instead of collapsing into a single-word column.
- Wide screens show the full action cluster beside the identity content.
- Compact desktop and tablet widths show the prioritized controls and secondary action strip.
- Mobile widths keep the primary action visible and expose every secondary action through the
  keyboard-accessible overflow menu.
- The desktop sidebar becomes an accessible navigation drawer on mobile, leaving the page a
  readable content width.
- Long text wraps without horizontal page overflow.
- Wide desktop mastheads retain right-aligned actions and the established visual hierarchy.
- All consumers of the shared masthead follow the same responsive behavior without page-local
  width overrides.
