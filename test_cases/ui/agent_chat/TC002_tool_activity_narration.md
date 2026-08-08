# TC002: Tool Activity Narration and Replay

## Description

Verify that multi-tool activity renders once per execution batch with human-readable narration and
stable state across live streaming, reload, and backward event pagination.

## Preconditions

- DB-backed stack is running with a tool-calling model
- An agent session can call tools from at least three families, including a structured-result tool
- Browser viewport can be tested at desktop and mobile widths

## Test Data

| Field | Value |
|-------|-------|
| Repeated batch | Two distinct read-only calls from one tool family in one model iteration |
| Mixed batch | One read/search call plus one execute/write call |
| Structured result | Object or array result with a meaningful count, message, or summary |

## Steps

1. Open the session before sending the prompt so activity arrives over SSE.
2. Send a prompt that causes a single call, a meaningful repeated-call batch, and a mixed batch.
3. Expand the turn work log and verify every actual batch appears once in event order.
4. Expand a repeated batch and verify both distinct calls remain as child rows.
5. Verify collapsed previews use concise summaries and never show escaped JSON such as `"{\n`.
6. Expand **Details** and verify structured machine output remains available and secret-like fields
   are masked.
7. Reload the page. Verify the same groups, headlines, completion states, and default collapsed state.
8. Load older events across an activity boundary. Verify the provisional activity enriches into one
   group rather than disappearing or duplicating.
9. Repeat at a 390 px viewport. Verify headlines wrap, controls remain reachable, and no horizontal
   overflow appears.
10. Use keyboard navigation on group and Details controls; verify focus, `aria-expanded`, and the
    controlled content update together.

## Expected Result

- One activity group exists per distinct `exec_id`; retries with the same `exec_id` reconcile into
  that group, while distinct executions remain distinct.
- Exact REST/SSE event replays do not duplicate groups or rows.
- Repeated calls are summarized once at group level and remain individually inspectable.
- Started and completed narration use the correct tense and capability-owned names.
- Human previews prefer safe structured summaries; raw JSON is confined to Details.
- Live, reload, pagination, desktop, and mobile render the same completed structure.
- Specialized interactive tool cards such as connection setup remain interactive.
