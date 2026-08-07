# TC001 Edit-Tab Agent Checks

## Description

Verifies that advisory agent checks are discoverable and actionable while editing, while Preview
contains only the rendered system prompt, tools, and initial files.

## Preconditions

- Development stack is running.
- At least one editable agent exists.
- Utility LLM is configured to verify deeper analysis and proposed fixes.

## Test Data

| Field | Value |
|---|---|
| System prompt with built-in finding | `Help the user named {{customer_name}}.` |

## Steps

1. Open an editable agent and select Edit.
2. At desktop width, confirm the right rail orders cards as Capabilities, Checks, then Health check.
3. Change the system prompt to the test value and wait for built-in checks to refresh.
4. Confirm the template-variable finding appears without saving the agent.
5. Select Analyze and wait for the deeper AI review to finish.
6. If a finding proposes a system-prompt replacement, select Apply fix and confirm the editor text changes while the agent remains unsaved.
7. Select Preview.
8. Confirm Preview shows Full System Prompt, Available Tools, and Initial Files without Checks or Health check cards.
9. Resize to a narrow viewport and confirm the shared columns stack Agent Details before
   Capabilities, Checks, and Health check; repeat the Edit/Preview switch.

## Expected Result

- Checks and Health check are visible in the Edit rail below Capabilities and remain advisory.
- Built-in findings refresh from the current unsaved configuration.
- Analyze exposes a loading state and then findings or a clear error without losing built-in findings.
- Apply fix updates only the authored form state; saving remains explicit.
- Preview is limited to the resolved system prompt, tools, and initial files.
- Cards and actions remain usable without horizontal clipping at narrow widths.
