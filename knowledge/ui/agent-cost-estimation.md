---
type: Decision
title: "Agent Cost Estimation"
description: "How the agent UI estimates model spend before execution using explicit workload scenarios, assembled definitions and catalog pricing."
tags:
  - everruns
  - ui
  - agents
  - cost
---

# Agent Cost Estimation

## Intent

An agent definition fixes some context and model choices, but cannot determine how much work
a future task requires. Give users a useful planning calculator without implying a quote,
spending cap or measured confidence. Entry: **Estimate cost** in the header of the
[Agent Page](agent-page.md), visible from every tab and in edit mode. The side sheet uses the current draft and never saves or executes it.

## Presentation

The default view asks only what kind of task the user expects and how many tasks they run per day.
Task choices use everyday examples, and headline amounts round to cents (positive amounts below
a cent stay explicitly below $0.01). Token counts, model-call assumptions, cost breakdowns and
the detailed method live behind **Advanced assumptions**, closed by default. Keep the AI-only
pricing scope visible without requiring that disclosure.

## Strategy

- Resolve the model in runtime order: agent override, nearest model in the harness ancestry,
  organization default. Missing explicit references or incomplete ancestry must not silently
  price a different model. Price data comes from the existing model catalog, not a second table.
- Fetch the assembled definition through the existing preview API. Approximate tokens for its
  full prompt, tool schemas and a message-overhead allowance. Text sizing is a heuristic, with
  more allowance for non-ASCII text; it is not a provider tokenizer. Preview failure withholds
  the estimate, and retry is explicit. Query cache entries include organization and draft.
- Define a task as a fresh conversation. Users choose an illustrative workload or enter calls,
  starting input, output per call (including reasoning), and new tool-result tokens per call.
  Resend the definition and accumulate previous outputs and tool results on subsequent calls.
  This makes multi-step work more expensive than multiplying one call's cost by call count.
- Apply catalog context tiers independently to each call, with the same strict threshold
  semantics as runtime usage pricing. Honor agent/model output ceilings. Do not manufacture a
  compaction strategy when a scenario exceeds the input or context limit: flag it instead.
- Compute lighter and heavier scenarios by changing calls, output and tool-result volume.
  These are sensitivity examples, not observed percentiles or a confidence interval. Neither
  scenario bounds actual spend. Keep the starting definition and input fixed across scenarios.
- Project the same task workload across the user-entered daily volume and a stated 30-day month.
  Zero volume is valid. Invalid or empty fields must not produce stale totals. Unknown prices
  are unavailable, not zero; tiny positive amounts must not render as free.

## Scope and calibration

These are catalog token costs for one model. Cache discounts are not assumed; cache-write
premiums, retries, media, paid tools, sandboxes, subagents, additional models and platform fees
can change the bill. Files, retrieved knowledge and user attachments must be accounted for in
starting input or tool-result assumptions when they enter context. Session/turn model overrides
are outside an agent-level forecast. Subscription providers may have a different billing basis.

Use representative Playground tasks and recorded usage to tune the workload assumptions before
budgeting. Do not automatically seed predictions from lifetime aggregate session stats: they mix
agent revisions, models, failed work, idle sessions and multiple tasks per conversation. An
empirical estimator would need comparable completed task samples, configuration provenance,
pricing coverage and a visible sample size before claiming measured ranges. This calculator
deliberately exposes its assumptions until those data contracts exist.

## Sources and verification

- [Calculator](../../apps/ui/src/lib/agent-cost-estimate.ts)
- [UI](../../apps/ui/src/components/agents/agent-cost-estimate.tsx)
- [Calculation tests](../../apps/ui/src/__tests__/agent-cost-estimate.test.ts)
- [Interaction tests](../../apps/ui/src/__tests__/agent-cost-calculator.test.tsx)
- [Manual case](../test-cases/ui/agent_cost/TC001_estimate_agent_cost.md)
