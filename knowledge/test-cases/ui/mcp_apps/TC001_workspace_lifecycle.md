---
type: Test Case
title: "TC001: MCP Apps - Workspace lifecycle"
description: "Verify explicit session rendering, preserved presentation state, connection recovery and organization isolation in an MCP Apps host."
tags:
  - everruns
  - test-case
  - ui
  - mcp
---
# TC001: MCP Apps - Workspace lifecycle

## Description

Verify explicit session rendering, preserved presentation state, connection recovery and organization isolation in an MCP Apps host.

## Preconditions

- A running deployment with its configured frontend, connected to an MCP Apps host that supports embedded frames and server tool calls.
- An authenticated MCP user with session read/write permission and membership in two organizations. Also retain a read-only PAT for the denied-write check.
- A fresh session per organization, with an unanswered question in the first. Record the organization IDs, session IDs and pending question's tool-call ID.
- A protocol harness may inject delayed or missing host replies. Record whether validation used that harness or an actual ChatGPT/Codex host.

## Test Data

| Value | Data |
| --- | --- |
| Draft answer | Use staging after the release review |
| Scope | The two distinct organization and session IDs created above |

## Steps

1. List tools. Confirm session data tools have no UI resource, `session_open` has a versioned UI resource, and app-only actions have no render template.
2. Call `session_get_status` for the fresh session. Confirm it returns data without opening a workspace. Call `session_open` and inspect that session in the workspace.
3. Select Approvals, enter the draft answer without submitting, and record the pending question's tool-call ID. Deliver another entry result for the same session, then change host theme and enter fullscreen. Confirm the selected tab and draft remain, and no answer was sent.
4. Answer the question. Re-read its recorded pending call and session events to confirm that exact question resumed once. Repeat with read-only credentials: the action must be denied and the question must remain unanswered.
5. Suppress the initial host initialization reply for 30 seconds. Click Retry connection after the timeout, restore replies, and confirm the original requested view opens. No mutation should be replayed.
6. Delay a workspace data reply, switch to the other organization, then release the old reply. Confirm no old data, cached queries or drafts appear in the second organization. A new request must carry its organization ID.
7. Close a workspace with an outstanding request. Release the late reply and confirm no further child response or live polling appears. Inspect the persisted record to determine whether any already-admitted write completed; a timeout must never cause automatic resubmission.

## Expected Result

Data calls remain usable without UI. Rendering is explicit. Repeated results and host context changes preserve presentation state, while organization changes discard it. Connection initialization can recover through a user retry, and late replies cannot cross workspace lifetimes or organizations.
