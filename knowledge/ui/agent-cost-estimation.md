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

The header opens a smart planning dashboard. The host utility LLM infers three representative
tasks from the assembled agent instructions and tool definitions, with optional user guidance.
Generated tasks are hypotheses, never measured usage or inferred frequencies. Users edit the
relative mix (equal initially), choose tasks per day or active users times tasks per user per day,
and see average task, daily and 30-day monthly costs plus a scenario chart.

Volume and mix sliders recompute locally without LLM calls. Generated plans are cached by
organization, draft, guidance and display locale for ten minutes; they are not persisted.
Changing provenance resets local edits. Explicit regeneration replaces assumptions. UI states
clearly separate loading, unavailable inference and unknown pricing. The manual calculator remains
available when the utility service is disabled, limited or fails.

Advanced assumptions expose per-task model calls, input, output and tool-result sizes. Authored
examples remain outside translation. The deployment utility model produces natural-language
suggestions in the selected display locale. User model selection still determines prices.

## Bounded inference

The purpose-specific [cost planning command](../../crates/server/src/domains/agents/cost_plan.rs)
requires agent management permission and resolves previews through the same tenant and harness
permission boundary as ordinary previews. It shares analysis rate and concurrency admission,
bounds definition size, completion time and output, and validates strict structured workloads.
The utility model receives neither credentials nor executable tools. It cannot choose prices,
change agents, or execute representative tasks. Provider failures return generic errors.
Suggestions can still be mistaken or steered by authored text; every assumption remains editable.

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
- Calculate each selected task separately, then weight its cost by the user-selected mix. Never
  average token counts before applying tiered prices. Project that mix across daily volume and a
  stated 30-day month.
  Zero volume is valid. Invalid or empty fields must not produce stale totals. Unknown prices
  are unavailable, not zero; tiny positive amounts must not render as free.

## Scope and calibration

The default remains catalog token costs for one model. Smart planning also accepts explicit
cache-read/write shares and a model-only retry allowance (zero initially). Shares apply to input
tokens and total at most 100%; tier selection still uses the full prompt. Cache buckets follow
runtime pricing fallbacks. Retries are a spend allowance, not an inferred failure probability.

Additional resource lines describe average usage per task or fixed monthly fees. Users supply
sandbox instance prices, paid-tool rates, media costs, per-child-task budgets, and hosting fees;
the current utility planner does not infer these quantities. Unknown prices make the affected
total unavailable. Zero-priced rates remain valid. Monthly fees survive zero task volume and
are allocated across 30 days only for the daily headline. Scenario shading varies model work,
not resource quantities. The total covers included resources, never every possible bill.

The [pricing hook](../../apps/ui/src/hooks/use-agent-cost-pricing.ts) is a host override seam.
It classifies model spend as prepaid credits or external, and supplies current service rates.
Self-hosted defaults to provider billing with no managed rate card. A host supplies its credit
label and effective rates without putting host billing logic in OSS. Rates are re-resolved by
stable identity when refreshed; a missing or failed rate cannot retain a stale zero/price.
Client resource inputs never book money or change provider billing. Credits are USD-denominated
planning amounts, distinct from external bills; no arbitrary credit conversion is invented.

Files, retrieved knowledge and user attachments must be accounted for in
starting input or tool-result assumptions when they enter context. Session/turn model overrides
are outside an agent-level forecast. Subscription providers may have a different billing basis.

Use representative Playground tasks and recorded usage to tune the workload assumptions before
budgeting. Do not automatically seed predictions from lifetime aggregate session stats: they mix
agent revisions, models, failed work, idle sessions and multiple tasks per conversation. An
empirical estimator would need comparable completed task samples, configuration provenance,
pricing coverage and a visible sample size before claiming measured ranges. This calculator
deliberately exposes its assumptions until those data contracts exist.

## Shared announcement

The original `2026-10-28-agent-cost-estimation` changelog link was shared publicly. At the
owner’s request, the docs Worker permanently redirects its HTML and Markdown URLs to
`2026-10-28-smart-agent-cost-projections`, preserving query parameters. The original dated
source remains marked superseded. This announcement redirect is covered by the
[Worker tests](../../apps/docs/worker/index.test.mjs).

## Sources and verification

- [Calculator](../../apps/ui/src/lib/agent-cost-estimate.ts)
- [UI](../../apps/ui/src/components/agents/agent-cost-estimate.tsx)
- [Calculation tests](../../apps/ui/src/__tests__/agent-cost-estimate.test.ts)
- [Interaction tests](../../apps/ui/src/__tests__/agent-cost-calculator.test.tsx)
- [Manual case](../test-cases/ui/agent_cost/TC001_estimate_agent_cost.md)
