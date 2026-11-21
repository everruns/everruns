---
type: Specification
title: "Session Spend"
description: "What a session spent in dollars, for the session itself: get_session_info's spend, its sources (model calls, background calls, images, sandboxes, subagents), pricing basis, privacy and which hosts provide it."
tags:
  - everruns
  - execution
  - capabilities
  - cost
---

# Session Spend

Status: implemented. Shape and pricing: `crates/contracts/src/runtime/session_spend.rs`;
sandbox list prices: `crates/contracts/src/sandbox_prices.rs`; control-plane read:
`crates/server/src/domains/session_cost/spend.rs` (`worker_get_session_spend`); tool:
`get_session_info` in `crates/core/src/host/session_services/capabilities/session.rs`.
Public page: `docs/capabilities/session.md`.

## Why

Agents were asked to watch their own cost (`self_budget`, a PR comment that says what a
change cost) but `get_session_info` returned tokens only, and on hosted workers not even
those. Tokens cannot be added across models or turned into dollars by the agent. The
goal is a realistic dollar figure: exact where a provider reports it, a labelled
estimate otherwise.

## What it counts

| Part | Source | Priced by |
|---|---|---|
| Model calls in turns | `llm_generations` (v1) or the cell's log (v2, `spend` read) | provider-reported cost, else the model price table |
| Background calls (titles, compaction, dreaming) | the same rows with no turn | as above |
| Images (`generate_image`, `edit_image`) | the tool results' usage | the image model's price table |
| Sandboxes | lifecycle spans (`sandbox_state_transitions`) | org price list (`sandbox.time`), else the provider list price |
| Subagents | each child's own spend, two levels, 100 per session | as above |

`wasted_usd` is the cost of calls in failed turns, `current_turn_usd` the asking
turn's share. `basis` is the weakest basis of any part; `partial` says some part had no
price (an unknown model, a sandbox provider without a public rate).

Not counted, and listed in `not_included`: paid tools on the org's own keys (search,
browser, fetch rendering; counted elsewhere, not priced), knowledge indexing,
observers and evals, and Everruns' own infrastructure.

## Sandbox list prices

No sandbox provider reports per-sandbox cost: Daytona's API exposes quota, E2B none,
boxd quotes plans by email. Modal's billing report (checked live 2026-11-20) is per App
and hour, and every sandbox runs in one App, so it cannot be split per session. Cost is
therefore computed as providers bill it: reserved resources from the pinned profile x
time in each state x the published rate, effective-dated so old sessions keep their
rate. An org rate wins (discounts, flat plans) and covers running time only; Daytona's
stopped-disk charge is added at list price.

## Privacy

The figure is the session's own and its subagents'. The read takes only the asking
session (`owned_session`), so an agent cannot ask for another session, and never
returns org totals, budgets or usage policies, credits, balance or markup, the org's
price list, or provider accounts and keys. On the managed demo it is the upstream model
cost, not credits. There is no org switch to hide it: the same numbers are on the
session's Cost tab for anyone who can view the session.

## Hosts

| Host | Spend |
|---|---|
| v1 worker (gRPC) and in-process server | internal command over the worker link |
| Cell (data plane v2) | host call `session_spend`; the control plane reads the cell's log and prices it |
| Embedded framework, Yolop | none yet: `spend` is `null`, tokens only |

A failed read returns `spend: null` and the tool still succeeds.

## Limits

- Retries inside a turn that later completes count as normal spend; only failed turns
  are wasted.
- v2 sandbox spans live in the control plane only when the sandbox provider reports
  lifecycle there; otherwise sandbox time is 0.
