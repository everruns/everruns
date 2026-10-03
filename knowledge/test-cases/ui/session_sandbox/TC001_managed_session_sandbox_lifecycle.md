---
type: Test Case
title: "TC001: Managed Environment - Recovery"
description: "Verify one provider-neutral Coding harness, stable tools, durable workspace recovery, and process-loss signaling on Daytona."
tags:
  - everruns
  - test-case
  - ui
  - environment
---
# TC001: Managed Environment - Recovery

## Description

Verify that a Coding Agent with a managed Daytona Environment uses the stable
tool vocabulary, checkpoints `/workspace`, and continues after its physical
sandbox is deleted.

## Preconditions

- Canonical local stack running with `AUTH_MODE=none`
- Valid Daytona connection in **Settings > Connections**
- LLM provider configured
- Coding harness imported as `coding`
- Active Agent using the Coding harness

## Steps

1. Open the Agent, select **More > Environments**, add a Daytona profile named
   `build`, make it the default, and save the Agent.
2. Open **Chats > New chat**, select the Agent, verify `build · daytona` is
   selected under **Environment**, and start the chat.
3. Ask it to create `/workspace/recovery-proof.txt` with a unique sentence and
   read the file back.
4. Verify the transcript uses `write_file`, `read_file`, and/or `bash`, with no
   `daytona_*` or `sandbox_*` model tool calls.
5. Open the Workspace Environment panel and record the logical Environment id,
   physical instance id, and generation.
6. Delete only the physical Daytona sandbox outside Everruns. Do not delete the
   logical Environment.
7. Ask the same session to read `/workspace/recovery-proof.txt` and run `pwd`.
8. Verify the tool call succeeds without manual create/resume, the file content
   is unchanged, the physical instance id changed, and the generation advanced.
9. Verify the event stream contains `environment.instance_lost` followed by
   `environment.recovered`, with `process_state_lost: true`.

## Expected Result

The session continues on a replacement physical sandbox. `/workspace` is
restored from the authoritative checkpoint, while process state is explicitly
reported as lost. The harness and tool names do not change.

## Cleanup

End the session and allow the normal Environment lifecycle to clean up the
replacement instance.
