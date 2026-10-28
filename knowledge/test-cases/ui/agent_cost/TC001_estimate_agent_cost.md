---
type: Test Case
title: "TC001: Estimate agent cost"
description: "Verify agent cost planning uses the draft model and assembled context, responds to workload assumptions and does not execute or save the agent."
tags:
  - everruns
  - test-case
  - ui
  - agents
  - cost
---

# TC001: Estimate agent cost

## Preconditions

- A local stack with an accessible agent and a model with catalog token pricing.
- A second model with different prices, and a model without catalog pricing.
- A harness with a model override and a child harness inheriting it.

## Test Data

| Field | Value |
|---|---|
| Route | `/agents/{agentId}` |
| Workload | Multi-step task |
| Daily volumes | 10, 20, 0 |
| Viewports | 1440 × 1100, 390 × 844 |

## Steps

1. Open **Estimate cost** in the page header. Check the resolved model, cost per task,
   monthly projection, scenario range and input/output breakdown.
2. Change daily volume from 10 to 20, then 0. Monthly totals double, then reach zero;
   task cost does not change.
3. Choose Quick answer, Multi-step task and Deep research. Verify estimates update. Confirm
   technical controls are hidden until opening **Advanced assumptions**. Open it.
4. Clear calls, then enter a negative number and a fraction. Verify no stale task quote remains.
5. Enter enough calls/input to exceed the context window. Verify an explicit limit message.
6. Close the sheet, edit the agent prompt/model without saving, and reopen. Verify the new
   draft is priced. Discard and reopen; saved configuration is priced again.
7. Clear the agent's model override and use the child harness. Verify its ancestor's model
   is priced. A missing explicit model must not fall back to the organization model.
8. Select a model without pricing. Verify unavailable pricing rather than a zero-cost claim.
9. Interrupt the preview request. Verify loading, then failure and Retry; restore access and retry.
10. Switch to Stats and confirm the header still offers **Estimate cost**. Inspect at both viewports in light and dark themes. Tab through controls and close the
    sheet using the keyboard. Confirm readable values and no horizontal overflow.
11. Inspect network requests and the agent's saved state. No estimator interaction creates
    a session, calls an LLM or updates the agent.

## Expected Result

- Estimates use the current model, assembled definition and visible workload assumptions.
- Unknown prices, failed requests, invalid input and context overflow never look like free work.
- Assumptions explain model-only pricing, sensitivity ranges and excluded costs.
- Estimation is usable on mobile and desktop and leaves saved configuration unchanged.
