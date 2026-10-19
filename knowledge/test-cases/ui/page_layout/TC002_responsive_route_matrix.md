---
type: Test Case
title: "TC002: Responsive route matrix"
description: "Verify that first-party routes keep their content and actions reachable across the supported narrow and desktop widths."
tags:
  - everruns
  - test-case
  - ui
  - page-layout
---
# TC002: Responsive route matrix

## Description

Verify that first-party routes keep their content and actions reachable across the supported narrow
and desktop widths. Check shared layout behavior first, then the representative consumers below.

## Preconditions

- A canonical local stack or authenticated test deployment is available with a selected test org.
- Representative agents, harnesses, sessions, virtual users, memories, and knowledge indexes
  exist so list, detail, edit, and session subroutes contain real data.

## Test Data

| Surface | Representative routes |
|---|---|
| Landing and reporting | `/chats`, `/reports` |
| Lists | `/sessions`, `/agents`, `/harnesses`, `/virtual-users`, `/skills`, `/memory`, `/knowledge-indexes`, `/models`, `/capabilities`, `/plugins`, `/evals`, `/observers` |
| Detail and edit | Representative agent, harness, virtual user, memory, knowledge index, capability, provider, and session detail/edit routes |
| Create forms | New agent, harness, virtual user, declarative capability, eval, observer, and agent-channel routes |
| Session and chat | `/chats`, `/playground`, plus read-only session Trace, Work, Events, Workspace, and Cost; legacy chat/transcript/timeline links redirect to Trace |
| Settings | Organization, members, MCP catalog, My agent experience, personal access tokens, profile, features, and payments where available; providers live under `/models/providers` |
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
   Record unavailable routes and missing fixture/permission prerequisites as blocked rather than passed.
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
