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
11. Add **Sandbox runtime** in smart planning. Leave its price blank: the total must be unavailable,
    with a price-needed message. Enter 5 minutes at $0.01/minute; at 100 tasks/day it adds $150/month.
12. Add a $10 monthly hosting fee, then set volume to zero. The $10 remains; per-task costs do not
    accrue. Confirm the chart includes that fixed fee rather than starting at zero cost.
13. Set a 10% retry allowance: only model spend increases. Cache reads/writes totaling over 100%
    withhold the projection. Restore valid assumptions.
14. With host pricing, verify managed service/model costs appear under credits and BYOK/external
    lines separately. Fail rate loading or remove a selected rate: no stale price may remain.
15. Inspect network requests and saved state. Smart suggestion generation calls the utility LLM;
    sliders, resource inputs and model adjustments do not. No session is created or agent saved.

16. Enable Chat workspace in Adoption. Open the planner above an Agent with an
    unsaved draft. Ask Chat to read and change guidance, volume, task assumptions
    and cache/retry allowances. Before/After and Undo apply only to the estimate;
    the underlying Agent, saved revision and billing remain unchanged. No extra
    cost-plan request occurs until clicking Update estimate. Clear a cache/retry
    input, edit a different field through Chat, and verify the empty input stays
    empty and remains repairable. Invalid frequency slider positions must fail.
17. Add an external resource row manually. Stage its quantity and external unit
    price in Chat. Add a host-priced row: its quantity is editable, its price and
    billing classification are read-only. Unknown rows and fields must fail.
    Adding/removing a row expires the preceding resource-input binding.
18. Switch to Manual calculator. The smart guidance/projection bindings must fail;
    read and edit every manual assumption and daily volume. Review and Undo are
    local and must not suggest Save. Later manual edits block conflicting Undo.
19. Close and reopen the drawer, change the Agent draft or switch organization.
    Captured bindings must fail. Closing restores the Agent's page context and
    its preceding unsaved fields. Repeat with Adoption off and at both viewports.

## Expected Result

- Estimates use the current model, assembled definition and visible workload assumptions.
- Unknown prices, failed requests, invalid input and context overflow never look like free work.
- Assumptions explain included resource costs, external billing, cache/retry allowances and scenario limits.
- Estimation is usable on mobile and desktop and leaves saved configuration unchanged.

## Smart projections

With the utility service configured, open Estimate cost and wait for three agent-specific
suggestions. Confirm the agent is not saved and no session is created. Adjust Tasks per day
with both slider and number input: daily/monthly projections change with no cost-plan request.
Switch to By users and verify active users times tasks per user per day determines volume.
Change the relative task mix, disable all tasks, and restore one: unavailable totals recover.
Empty, fractional and out-of-range inputs never retain valid monthly projections.
Edit per-task assumptions and verify oversized tasks withhold their totals. Change agent draft
or organization and confirm old suggestions/edits do not appear. Regenerate with workload
guidance, test retry on failure and Manual calculator when utility inference is disabled.
Check mobile, dark mode and a non-English locale. Check the old changelog URL redirects to the
new entry for HTML and Markdown readers, preserving query parameters.
