---
type: Test Case
title: "TC001: Custom workers - Existing chat and trace UI"
description: Verify the ordinary Playground chat and session trace render real custom-worker tool results and final answers without a separate UI.
tags:
  - everruns
  - test-case
  - ui
  - custom-workers
---

# TC001: Custom workers - Existing chat and trace UI

## Description

Verify the ordinary Playground chat and session trace render real custom-worker
tool results and final answers without a separate UI.

## Preconditions

- The [real-model smoke](../../agents/custom_workers/TC001_custom_data_worker_smoke.md)
  passed on a local stack with the UI and Caddy running.
- Record the fresh session id from that run, rather than selecting an old fixture.
- Development `AUTH_MODE=none` or an authenticated user in the session's org.
- The smoke's credential has been disabled after completion; this case reads
  completed results and does not send additional input.

## Test Data

| Field | Value |
| --- | --- |
| Chat route | `/playground/{session_id}` |
| Trace route | `/sessions/{session_id}/trace` |
| Expected tool | `sales_summary` |
| Final answer | Three north-region orders, 28,512 revenue cents |

## Steps

1. Open the fresh Playground session, wait for network idle and capture a screenshot.
2. Confirm user messages, completed Sales Summary activity, final answers and the
   ordinary message composer render without an error or special worker surface.
3. Expand the activity details and verify the database result, including the
   region, orders and revenue. Confirm completed work is not stuck running.
4. Open the session trace, wait for network idle and capture another screenshot.
5. Confirm three turns, nested real model calls, successful `sales_summary` steps,
   token usage and final answers. Check the updated aggregate belongs to the later
   turns and the first turn reports the original aggregate.
6. Compare the UI's session/turn identity and results with the smoke's canonical
   `events.json` and `work-report.json`. Record the fixture id and evidence paths.

## Expected Result

The existing Playground and trace components render the custom execution record,
including usage, successful tool activity and changed private data. No custom
frontend is needed. A legacy `/sessions/{id}/chat` redirect goes to its recording;
mutable chat with a selected agent lives in Playground, while `/chats` is the
managed Platform Chat surface.
