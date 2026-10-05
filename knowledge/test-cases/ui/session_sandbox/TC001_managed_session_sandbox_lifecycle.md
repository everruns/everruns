---
type: Test Case
title: "TC001: Managed Sandbox - Recovery"
description: "Verify one provider-neutral Coding harness, stable tools, durable workspace recovery, and process-loss signaling on Daytona."
tags:
  - everruns
  - test-case
  - ui
  - sandbox
---
# TC001: Managed Sandbox - Recovery

## Description

Verify that a Coding Agent with a managed Daytona Sandbox uses the stable
tool vocabulary, checkpoints `/workspace`, and continues after its physical
sandbox is deleted.

## Preconditions

- Canonical local stack running with `AUTH_MODE=none`
- Valid Daytona connection in **Settings → My agent experience**
- LLM provider configured
- Coding harness imported as `coding`
- Active Agent using the Coding harness

## Steps

1. Create a Daytona Sandbox Template named `build`, then configure the Agent's
   primary Sandbox to use that template.
2. Press **Test in Playground**, verify the Agent and `build · daytona`
   Sandbox are selected, choose a virtual user, and start the Playground chat.
3. Ask it to create `/workspace/recovery-proof.txt` with a unique sentence and
   read the file back.
4. Verify the transcript uses `write_file`, `read_file`, and/or `bash`, with no
   `daytona_*` or `sandbox_*` model tool calls.
5. Open the Workspace Sandbox panel and record the logical Sandbox id,
   physical instance id, and generation.
6. Delete only the physical Daytona sandbox outside Everruns. Do not delete the
   logical Sandbox.
7. Ask the same session to read `/workspace/recovery-proof.txt` and run `pwd`.
8. Verify the tool call succeeds without manual create/resume, the file content
   is unchanged, the physical instance id changed, and the generation advanced.
9. Verify the event stream contains `sandbox.instance_lost` followed by
   `sandbox.recovered`, with `process_state_lost: true`.

## Expected Result

The session continues on a replacement physical sandbox. `/workspace` is
restored from the authoritative checkpoint, while process state is explicitly
reported as lost. The harness and tool names do not change.

## Cleanup

End the session and allow the normal Sandbox lifecycle to clean up the
replacement instance.
