# TC001: Work View - Cross-Session Delegation Tree

## Description

Verify the cross-session **Work** view (EVE-756) lists an org's tasks from `GET /v1/tasks` grouped by `root_session_id` as a delegation tree, updates chips live from `task.updated` without a manual refetch, opens the shared task detail on selection, and renders a task whose kind is unknown at build time.

## Preconditions

- Server running (`just start-all` recommended)
- User logged in with an org that has (or can create) sessions
- LLM API key configured
- At least one session that spawns background work (subagent / handoff / background tool / monitor), ideally a delegation tree where a root session delegates to a child session so more than one `root_session_id` and a descendant session are present

## Test Data

| Field | Value |
|-------|-------|
| Nav item | Work (sidebar, top section) |
| Route | `/work` |
| Seed message | Delegate a subagent task and a background tool from this session, then wait. |

## Steps

1. In a session, send the seed message so the harness creates at least one background task; if possible, have it delegate to a child session so a descendant-session task exists.
2. Click **Work** in the sidebar (top navigation).
3. Confirm the page header reads **Work** with the "Tasks across every session, grouped by delegation tree" subtitle.
4. Observe the delegation tree: each root session renders as a card with a "Session …" link and an "N tasks" count, and one chip per task (kind icon, display name, state color).
5. For a tree with a descendant session, confirm its tasks render under a "↳ via session …" sub-label within the same root card.
6. While a task is still running, let its state advance (progress / `state_detail` / terminal state) and confirm its chip updates in place **without reloading the page**.
7. Click a task chip.
8. Confirm the detail pane shows the shared task card: kind badge, ID, created time, progress/summary/error, result + artifact links, Cancel (for non-terminal), and a "Details" expander.
9. Click **Details** and confirm the lifecycle timeline and message thread load.
10. If a task with an uncommon/unknown `kind` is present, confirm its chip still renders (generic icon + display name).
11. With no tasks in the org (fresh org), confirm the empty state "No work yet" renders.

## Expected Result

| Check | Expected |
|-------|----------|
| Navigation | "Work" appears in the sidebar and routes to `/work` |
| Grouping | Tasks grouped into one card per `root_session_id`; count is accurate |
| Descendant nesting | Descendant-session tasks appear under "↳ via session …" in the root card |
| Live update | A running task's chip reflects `task.updated` with no manual refetch |
| Selection | Clicking a chip opens the shared task detail card |
| Detail drill-down | Expanding shows lifecycle timeline + message thread |
| Unknown kind | A task with an unrecognized kind still renders from its snapshot |
| Empty state | "No work yet" shows when the org has no tasks |
| Design system | View uses Slate tokens only; no visual regressions |

## Cleanup

- Cancel or let finish any tasks started for the test.
