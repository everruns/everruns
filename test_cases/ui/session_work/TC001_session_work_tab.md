# TC001: Session Work tab — tasks, leased resources and schedules

## Description

Verify the session **Work** tab (`/sessions/{sessionId}/work`) lists the background work *this*
session started — subagent tasks, handoffs, background tools and monitors — updates chips live from
`task.updated` without a manual refetch, opens the shared task detail on selection, lists leased
resources, and renders the session's schedules as read-only rows.

This replaces the retired org-wide `/work` view (EVE-756). EVE-854 folded it into session detail and
EVE-855 removed the top-level route, so work is scoped to the recording that started it. A subagent
task links to its own recording from the detail card rather than nesting under a delegation tree.

## Preconditions

- Server running (`just start-all` recommended)
- User logged in with an org that has (or can create) sessions
- LLM API key configured
- At least one session that spawns background work (subagent / handoff / background tool / monitor)
- For the Schedules section, a session whose `features` include `schedules`

## Test Data

| Field | Value |
|-------|-------|
| Route | `/sessions/{sessionId}/work` |
| Nav item | **Work** tab in session detail |
| Seed message | Delegate a subagent task and a background tool from this session, then wait. |

## Steps

1. In a session, send the seed message so the harness creates at least one background task.
2. Open the session and click the **Work** tab.
3. Confirm the page renders a **Tasks** section, a **Resources** section, and — only when the
   session has the `schedules` feature — a **Schedules** section.
4. In **Tasks**, confirm one chip per task (kind icon or running spinner, truncated display name,
   state color), ordered newest first by `created_at`.
5. While a task is still running, let its state advance (progress / `state_detail` / terminal state)
   and confirm its chip updates in place **without reloading the page**.
6. Click a task chip.
7. Confirm the detail pane shows the shared task card: kind badge, ID, created time,
   progress/summary/error, result + artifact links, Cancel (for non-terminal), and a "Details"
   expander. Before any selection, the pane reads "Select a task to see its detail."
8. Click **Details** and confirm the lifecycle timeline and message thread load.
9. If a task with an uncommon/unknown `kind` is present, confirm its chip still renders (generic
   icon + display name).
10. For a subagent task, confirm the detail card links out to that subagent's own session recording.
11. In **Resources**, confirm each leased resource renders as a card with its display name, kind,
    status badge, registered time, and any status/progress/summary/log/result metadata.
12. In **Schedules**, confirm each schedule renders as an inert row — cadence, next trigger, fire
    count and an Active/Disabled badge — with **no** enable/disable/trigger/delete controls.
13. On a session with no background work, confirm the "No work yet" empty state and the "No
    resources leased" empty state render, and that a session that scheduled nothing reads "This
    session scheduled no work."

## Expected Result

| Check | Expected |
|-------|----------|
| Navigation | "Work" appears as a session detail tab and routes to `/sessions/{sessionId}/work` |
| Scope | Only tasks belonging to this session are listed; there is no org-wide grouping |
| Ordering | Task chips are ordered newest first by `created_at` |
| Live update | A running task's chip reflects `task.updated` with no manual refetch |
| Selection | Clicking a chip opens the shared task detail card beside the chip list |
| Detail drill-down | Expanding "Details" shows the lifecycle timeline + message thread |
| Subagent link | A subagent task's detail card links to that subagent's own recording |
| Unknown kind | A task with an unrecognized kind still renders from its snapshot |
| Resources | Leased resources render with status badge and metadata; empty state is "No resources leased" |
| Schedules | Schedules are read-only rows, and the section is absent without the `schedules` feature |
| Empty state | "No work yet" shows when the session started no background work |
| Retired route | The footer note points at `/sessions` for browsing work across sessions |
| Design system | View uses Slate tokens only; no visual regressions |

## Cleanup

- Cancel or let finish any tasks started for the test.
