---
title: Work Reports
description: See what an agent's turn is doing right now and what it got done, as one record built from the session's events and shown in chat, the sessions list and the API.
sidebar:
  label: Work Reports
appliesTo: [platform, cloud]
---

A **work report** answers two questions about a session: what is the agent
doing right now, and what did it get done. Everruns builds it from events the
session already records, so every agent has one without prompt changes or an
extra model call.

The same report appears in three places:

- **Chat** shows the plan the agent shared ("Step 2 of 4") above the work log
  while the turn runs, and a result box when it ends: "Done · 2 files changed",
  the agent's own summary, and links to what it produced.
- **The sessions list, the thread board and run cards** show the headline of a
  session that is still working, such as "Running `cargo test` 7 of 12".
- **The API** returns it as JSON.

## What a report says

| Field | What it holds |
|---|---|
| `state` | `working`, `needs_you`, `done`, `failed` or `stopped` |
| `headline` | One line: the running step, what the work waits on, or the result ("Done in 13s · 2 files changed") |
| `plan` | The checklist the agent shared, with `done`, `total` and the current item. Absent when the agent shared none |
| `steps` | The turn's tool calls, newest last, each labelled by what it did ("Ran `cargo test`, 3 of 12 tests failed") |
| `attention` | What the agent waits on: an approval, a question or another action |
| `result` | Facts measured from the run (files changed, tasks started, issues), a summary, checks, links |

Progress is never invented. "Step N of M" appears only when the agent shared a
plan with `write_todos`, `update_checklist` or `update_status`, and a step shows a
count only when its tool reported one. Without a plan the report lists steps and
draws no bar.

The summary comes from the agent when it called [`report_done`](/capabilities/task-management/#report_done),
and from the first sentence of its reply otherwise. `report_done` also records
how the agent checked the work and whether it could finish.

## Read a report

```bash
curl "$EVERRUNS_API/v1/sessions/$SESSION_ID/work" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY"
```

```json
{
  "report": {
    "turn_id": "turn_01933b5a00007000800000000000001",
    "state": "done",
    "headline": "Done in 2m 13s · 2 files changed",
    "steps": [
      {
        "id": "call_01",
        "tool_name": "bash",
        "label": "Ran `cargo test`, 12 tests passed",
        "state": "done"
      }
    ],
    "result": {
      "summary": "Fixed the proration rounding and added a test.",
      "summary_source": "agent",
      "checks": "Ran the 12 billing tests",
      "facts": [{ "kind": "files_changed", "value": 2 }]
    }
  }
}
```

The route returns the latest turn's report; pass `?turn_id=` for an earlier
one. A session that has not run a turn returns `{"report": null}`. From the
CLI, run `everruns sessions work get session_01h9`.

## In the Agent Execution API

`GET /v1/channels/{channel_id}/sessions/{session_id}` on an
[Agent API](/features/agent-api/) channel includes the latest turn's report as
`work`. What it shows follows the channel's visibility:

- `full`: the whole report.
- `activity` and `messages`: the public view. It keeps the state, plan counts,
  facts, the agent's summary and checks, and links. It leaves out step labels,
  plan item titles, approval prompts and file paths, because step labels are
  tool narration and can quote tool arguments. The headline becomes "Step 3 of
  4", "Working" or "Needs approval" while the work runs.

## Reporting progress from your own tools

A tool that knows how far along it is can send a count with its progress
message, and the report shows it on the running step. In the Framework, a tool
calls `ctx.progress_of("Rendering frames", 7, 12).await` on its
`ToolCallContext`.
