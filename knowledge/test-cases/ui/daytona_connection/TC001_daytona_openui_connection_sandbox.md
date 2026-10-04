---
type: Test Case
title: "TC001: Managed Environment - Daytona Connection"
description: "Verify that a Coding Agent's managed Environment requests a Daytona connection, resumes, and executes through stable tools."
tags:
  - everruns
  - test-case
  - ui
  - daytona-connection
---
# TC001: Managed Environment - Daytona Connection

## Description

Verify that a Coding Agent with a managed Daytona Environment prompts for a
Daytona API key when no connection exists, resumes after connection setup, and
executes through the provider-neutral tool surface.

## Preconditions

- Canonical stack running with PostgreSQL
- LLM provider configured
- Coding harness imported as `coding`
- Agent version has a default managed Daytona Environment profile
- **No** existing Daytona connection in Settings → My agent experience (disconnect first if present)
- Valid Daytona API key available for test

## Test Data

| Field | Value |
|-------|-------|
| Agent | Coding Agent with a managed Daytona Environment |
| Message | Write `56088` to `/workspace/daytona-proof.txt`, read it back, and report `pwd`. |
| Daytona API Key | *(use a valid Daytona API key)* |

## Steps

1. Run the Coding Agent to create a new session.
2. Send the test message.
3. Wait for the first managed Environment operation to request a connection.
4. Wait for the inline **Setup Connection** card to appear in the chat (OpenUI connection prompt for Daytona)
5. Click the **Connect** button on the inline card
6. In the API Key dialog:
   - Verify the dialog shows Daytona provider name and instructions
   - Enter the Daytona API key
   - Click **Submit**
7. Wait for the connection to be validated and the dialog to close
8. Wait for the interrupted turn to resume automatically.
9. Verify the transcript uses `write_file`, `read_file`, and/or `bash`, and
   never exposes `daytona_*` or `sandbox_*` tools to the model.
10. Verify the reply includes `56088` and `/workspace`.
11. Open the Workspace Environment panel and verify the managed Daytona
    Environment is running.

## Notes

- Environment lifecycle is control-plane-owned. The model is not given a
  provider-specific delete tool.
- Use the canonical PostgreSQL stack; the in-memory development store does not
  prove connection persistence or durable Environment recovery.

## Expected Result

| Check | Expected |
|-------|----------|
| Connection prompt | Inline "Setup Connection" card appears for Daytona provider |
| API Key dialog | Shows Daytona icon, provider name, and instructions |
| Connection saved | After submit, dialog closes and connection is stored |
| Environment created | Workspace panel shows the managed Daytona target |
| Stable tools | Transcript contains only provider-neutral execution/file tools |
| Command executed | Reply includes `56088` and `/workspace` |

## Cleanup

- End the session and allow normal Environment cleanup to release the physical sandbox.
- Disconnect the Daytona connection if it should not persist after the test.
