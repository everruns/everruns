# TC004: AGENTS.md Has No Effect with Base Harness

## Description

Verify that AGENTS.md is not read when using the Base harness, which does not include the `agent_instructions` capability.

## Preconditions

- Control-plane running

## Test Data

| Field | Value |
|-------|-------|
| Harness | Base (`harness_01933b5a000070008000000000000601`) |
| AGENTS.md Content | `Always respond with exactly one word.` |

## Steps

1. Create session with Base harness:
   ```bash
   curl -s -X POST "http://localhost:9000/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{"harness_id": "harness_01933b5a000070008000000000000601"}'
   ```

2. Write AGENTS.md (requires adding `session_file_system` capability to session first, or use direct storage).

3. Send a message.

4. Verify agent response does NOT follow AGENTS.md instructions (responds normally, not one word).

## Expected Result

| Check | Expected |
|-------|----------|
| Agent responds | Normal response, not constrained by AGENTS.md |
| Capability absent | Base harness capabilities do not include `agent_instructions` |
