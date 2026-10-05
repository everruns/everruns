---
type: Test Case
title: "TC001: Modal OpenUI Connection - Sandbox Lifecycle"
description: "Verify that an agent with the Modal capability prompts for a Modal token pair, creates a VM sandbox after connecting, executes a command, and terminates the sandbox."
tags:
  - everruns
  - test-case
  - ui
  - modal-connection
---
# TC001: Modal OpenUI Connection - Sandbox Lifecycle

## Description

Verify that an agent with the experimental Modal capability prompts for a Modal
token via the inline connection dialog when no connection exists, creates a VM
sandbox after connecting, runs a command in it, and terminates it.

## Preconditions

- Canonical stack running with PostgreSQL at development grade (Modal is
  experimental and not registered at production grade)
- LLM provider configured
- Seed agent **Modal Coder** available (or any agent with the `modal` capability)
- **No** existing Modal connection in Settings → My agent experience
- A valid Modal token pair (`ak-...` and `as-...`)

## Test Data

| Field | Value |
|-------|-------|
| Agent | Modal Coder |
| Message | Create a Modal sandbox, run `uname -r && echo 41077 > /workspace/proof.txt && cat /workspace/proof.txt`, then terminate it. |
| Token | `ak-...:as-...` |

## Steps

1. Start a session with the Modal Coder agent.
2. Send the test message.
3. Wait for the inline **Setup Connection** card for Modal.
4. Click **Connect**; verify the dialog shows the Modal name, the cloud icon,
   and a link to modal.com/settings/tokens.
5. Enter a malformed value (`not-a-token`) and submit; verify the dialog shows
   an error and stays open.
6. Enter the valid token pair as `ak-...:as-...` and submit.
7. Wait for the turn to resume and the agent to call `modal_create_sandbox`.
8. Verify the agent calls `modal_exec` and the output shows a kernel version
   that is not `4.4.0` (a VM, not gVisor) and `41077`.
9. Verify the agent calls `modal_manage_sandbox` with `terminate`.
10. In the Modal dashboard, open the `everruns-sandboxes` app and verify the
    sandbox is stopped.

## Expected Result

| Check | Expected |
|-------|----------|
| Connection prompt | Inline Setup Connection card for Modal |
| Malformed token | Rejected before saving, dialog stays open |
| Connection saved | Valid pair accepted, dialog closes |
| Sandbox | Created with runtime `vm`, `modal_exec` prints `41077` |
| Cleanup | Sandbox terminated; listed as stopped in Modal |
