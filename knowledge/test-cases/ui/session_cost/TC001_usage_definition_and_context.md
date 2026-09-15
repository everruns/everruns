---
type: Test Case
title: "TC001: Session usage, definition, and context"
description: "Verify that the session Cost tab reports token usage and failures and identifies the exact definition and context sources used by the run."
tags:
  - everruns
  - test-case
  - ui
  - sessions
  - cost
---
# TC001: Session usage, definition, and context

## Description

Verify that the session Cost tab reports token usage and failures and identifies the exact definition and context sources used by the run.

## Preconditions

- The full stack is running and the user is signed in.
- A session completed at least one LLM call with a known agent version, harness, model, and identity.
- The session used at least two context sections, such as rules and skills.
- A second session exists with no LLM call.

## Test Data

| Field | Value |
|---|---|
| Route | `/sessions/{sessionId}/cost` |
| Populated session | A completed session with recorded usage |
| Empty session | A session with no LLM generation |

## Steps

1. Open the populated session and select **Cost**.
2. Compare Prompt tokens and Completion tokens with the session's recorded LLM usage.
3. Compare Failed turns and Tool failures with the event timeline; if failed model calls exist, verify the model-call hint appears separately.
4. In **Definition**, verify agent and pinned version, harness, model ID, and identity match the session.
5. Follow the agent, harness, and identity links and confirm each opens the expected entity, then return.
6. If the session is a fork, follow **Forked from** and confirm the source session and sequence.
7. In **Context**, verify estimated tokens, context-window total, section item counts, and token totals.
8. Confirm Source breakdown is sorted by tokens and each percentage and section label match the report.
9. Open the empty session's Cost tab.

## Expected Result

- Usage totals and failure counters are derived from the selected session only.
- The definition identifies the exact agent version, harness, model, and identity used by the run.
- Context section and source totals agree with the context report and expose relative contribution.
- Entity and fork links preserve their recorded IDs.
- A session with no LLM call shows unavailable usage values and the explicit no-context-report state.
