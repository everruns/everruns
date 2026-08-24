# Responsive route matrix

## Description

Verify that first-party routes keep their content and actions reachable across the supported narrow
and desktop widths. Check shared layout behavior first, then the representative consumers below.

## Preconditions

- Everruns is running on the full DB-backed stack with authentication disabled.
- Representative agents, harnesses, sessions, apps, identities, memories, and knowledge indexes
  exist so list, detail, edit, and session subroutes contain real data.

## Test Data

| Surface | Representative routes |
|---|---|
| Landing and reporting | `/chats`, `/reports` |
| Lists | `/sessions`, `/agents`, `/harnesses`, `/agent-identities`, `/skills`, `/memory`, `/knowledge-indexes`, `/models`, `/capabilities`, `/mcp-servers`, `/plugins`, `/apps`, `/evals`, `/observers` |
| Detail and edit | Representative agent, harness, identity, memory, knowledge index, capability, app, provider, and session detail/edit routes |
| Create forms | New agent, harness, identity, declarative capability, app, eval, observer, and app-channel routes |
| Session and chat | `/chats` plus session chat, transcript, timeline, work, events, workspace, and cost |
| Settings | Organization, members, providers, connections, personal access tokens, profile, features, and payments |
| Durable execution | Overview, workers, workflows, queues, schedules, and circuit breakers |
| Auth and onboarding | Login, signup/register, password reset, verification, onboarding, connection completion, and invite error state |
| Developer previews | Developer index and each linked component showcase |

| Viewport | Size |
|---|---|
| Small mobile | 320 × 900 |
| Mobile | 375 × 900 |
| Mobile wide | 390 × 900 |
| Tablet | 768 × 1000 |
| Compact desktop | 1024 × 1000 |
| Desktop | 1440 × 1000 |

## Steps

1. At each viewport, open every reachable route in the matrix and wait for its data, empty, error,
   or loading state to settle.
2. Confirm the document has no horizontal page overflow. For intentionally wide tables or code,
   confirm horizontal scrolling is contained inside the component and can be reached by keyboard.
3. Check page shells, mastheads, breadcrumbs, descriptions, badges, tabs, filter/action rows, forms,
   and right rails. Confirm content wraps or stacks in priority order without clipping.
4. Check long names, identifiers, URLs, provider errors, and status descriptions. Confirm the full
   value remains available through the component's copy, title, or expanded-detail affordance.
5. Exercise every visible primary action and each overflow menu with pointer and keyboard input.
6. Recheck loading, empty, error, and populated list states on representative routes.
7. At desktop width, confirm responsive adaptations do not reduce established data density or hide
   actions that fit.

## Expected Result

- The page document never scrolls horizontally by accident.
- Mobile list records use labeled stacked content, responsive columns, or a deliberately scrollable
  region selected for the content semantics; desktop-shaped rows are not squeezed into unreadable
  columns.
- Titles, descriptions, statuses, tabs, forms, rails, and actions remain readable and reachable by
  touch and keyboard at every viewport.
- Long values cannot force their parent wider and retain an accessible full-value affordance.
- Empty, loading, and error states obey the same containment contract as populated content.
- Desktop layouts retain their normal density and action hierarchy.
