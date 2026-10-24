---
type: Specification
title: "Work reports"
description: "One runtime-built record of what a piece of work is doing now and what it got done, read by every surface."
tags:
  - everruns
  - execution
---
# Work reports

Status: approved 2026-10-23; building per the delivery plan below.

A **work report** answers two questions about a piece of work, for every
surface that shows it: what is it doing right now, and what did it get done.
The chat work log, the work card, the thread board, the sessions list, the API
and channel integrations all read the same record instead of each composing a
partial view from raw events.

The shape lives in
[`work_report.rs`](../../crates/contracts/src/runtime/work_report.rs); this
concept owns the intent and the rules.

## Why

Before work reports the parts existed but did not meet: tool narration said
what each call did, three tools carried a plan, approvals and questions parked
turns, and `turn.completed` carried raw counts. Each surface derived its own
view, finished steps said what was attempted rather than what came out, and
nothing composed a closing line with a hard fact ("Done in 13s · 6 files
changed").

## Rules

- **The runtime writes the report.** It is built from events the session
  already records, so every turn of every agent has one with no prompt change
  and no extra model call. The agent can enrich it, never has to.
- **One reducer.** `WorkReport::from_events` is the only place events become a
  report. Surfaces render it; they do not re-derive it, so the live view and a
  replay cannot disagree.
- **Computed on read.** `GET /v1/sessions/{id}/work` reduces the turn's events
  when asked; nothing new is stored. Clients refetch on the events they already
  stream. A second client-side reducer and a `work.updated` event were
  considered and rejected: two reducers drift, and a server-pushed snapshot
  needs per-session state on the worker for no gain over a cheap read.
- **No invented progress.** "Step N of M" exists only when the agent shared a
  plan. Without one the report counts steps and draws no bar.
- **Labels are narration.** Step labels come from
  [tool narration](tool-narration.md), which already applies the redaction
  rules, so the report adds no new path for arguments to leak.
- **Needs you points, never answers.** A parked turn's report names the open
  request; answering still goes through the endpoint that owns it (tool
  approvals, tool results).

## What feeds each part

| Part | Source |
|---|---|
| Headline | Running step, else the plan's in-progress item; the open request while parked; facts once finished |
| Steps | `tool.started` / `tool.progress` / `tool.completed`, one per call, newest kept |
| Plan | Latest successful call of a plan tool (`write_todos`, `update_checklist`, `update_status`); not a step |
| Needs you | Open `tool.call_requested` calls: approval, question, other client action |
| Facts | Distinct `file.written` paths, `task.created`, the turn's issue count, step count |
| Summary | First sentence of the final reply (`final_answer_preview`) |

A call parked on the approval gate is not a step and not an issue: it did not
run. The request replaces it, and the model's re-run after approval is the
step.

Turn terminal states map to report states: completed is done, failed is
failed, cancelled and sealed are stopped. Steps still running when a turn ends
are shown as cut short, not running.

## Surfaces

The work card appears in the chat run card, the coordinator thread board and
the sessions list. It is deliberately not on the Agents list: that page is a
fleet view for agent builders, where one agent can have a thousand sessions,
and it keeps its run counts and bars (user decision, 2026-10-23).

In the session view the latest turn's work log reads the report: its header
shows the live headline (the running step with its count, the plan's current
item, or what is waiting on the person), the plan is pinned under it while the
turn runs, and once done a box lists files changed, tasks started, links and
the agent's own summary. A summary taken from the reply is not repeated there,
since the reply follows. Older turns keep "Worked for 13s": one report request
per turn would cost a long session hundreds of reads.

Rows that show a session elsewhere (the chat run card for a delegated run,
the thread board, the sessions list) put the live headline in place of their
static status while that session runs or waits, prefixed with the plan step
when there is one. They poll the report only while live, so a list of a
thousand finished sessions costs no report reads.

## Delivery plan

1. Reducer, `GET /v1/sessions/{id}/work`, this concept.
2. Result-aware narration for common tool families and counted
   `tool.progress`, so steps say what came out.
3. Session view: work log header and done line read the report; pinned plan.
4. Shared work card for chat runs, the thread board and the sessions list.
5. `report_done`: the agent's own summary, checks and artifacts;
   `complete_assignment` folds into it.
6. Agent Execution API result, Slack, AG-UI and A2A projections, SDKs, docs.
