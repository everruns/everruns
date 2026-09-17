---
type: Specification
title: "Judgment Service"
description: "Internal typed-judgment service for capability internals."
tags:
  - everruns
  - operations
---
# Judgment Service

<!-- Design Decisions:
  - Modeled on the utility LLM service rather than as a model provider: same
    host-owned, deployment-configured, never-agent-configurable posture. The
    two are siblings, not layers.
  - The contract is provider-neutral (noul/choice/score), not TypeSafe-shaped.
    Core names none of the vendor; `everruns-host` owns the adapter. Swapping
    vendors, or answering judgments with a fine-tuned local model, is a host
    change only.
  - Answers are values, not text. The point is removing the parse step, not
    saving tokens: a guardrail that fails open on malformed JSON is a security
    control with a silent bypass, and this contract has no such path.
  - The deployment client does not retry. Its primary callers sit on
    latency-critical seams (pre-tool-use, end-of-message), where a retried
    round trip costs the user more than a fail-open costs the policy.
  - Batching is the contract, not an optimization: every question for a stage
    rides one request, because per-check round trips are what forced the
    utility-LLM path's per-invocation call caps.
-->

## Intent

Provide a system-owned service that answers **typed questions** for built-in
capability internals: a probability, a selected option, a position along
described levels.

It is the sibling of the [Utility LLM Service](utility-llm.md), for the cases
where a call site wants a *decision*, not prose it has to parse. Neither is an
agent model provider, a public API, a UI option, or a session/agent
configuration surface.

## Why a second service

The utility LLM answers with text. Every model-backed call site therefore
carries a prompt that begs for JSON, a parser, and a fallback for when the
parse fails. In a guardrail that fallback is fail-open — a malformed verdict
reads as *allow*.

A judgment service removes that class outright, and adds two things a chat
model cannot give cheaply:

- **Calibrated probabilities.** Guardrail moderation already asked the utility
  model to emit integers 0-100 per category and compared them against a
  configured threshold. That is a probability simulated by asking a text model
  to write a number. Here the probability is the answer, and the distribution
  comes with it, so an "any serious hit" rule can read the tail instead of a
  mean that hides a bimodal answer.
- **Batching.** Questions asked together are answered together, in parallel,
  over the same state. One stage costs one round trip regardless of how many
  checks it carries.

## Core Contract

`everruns-core` owns the abstraction ([`crates/core/src/judgment.rs`](../../crates/core/src/judgment.rs)):

- `JudgmentService` is the async trait used by capability internals.
- `JudgmentQuestion` is one of three primitives — `Noul` (probability of yes),
  `Choice` (one option plus its distribution), `Score` (a position across
  ordered levels plus its distribution).
- `JudgmentRequest` carries the state, an ordered list of `(id, question)`, and
  attribution metadata. Ids are for the caller's code and never reach the model,
  so every question must carry its full meaning.
- `JudgmentAnswer` exposes `probability_yes`, `confidence`, and
  `probability_at_or_above` — the last is the honest reading for a
  "did anything serious happen" rule.
- `JudgmentService::is_configured()` reports whether the deployment enabled it.
- `HostComposition` carries the active service; `ToolContext` and
  `PostGenerationOutputContext` thread it to capability hooks, alongside the
  utility LLM service.

A noul near 0.5 means yes and no are near-equally likely. It does not mean
"medium intensity", and it is not a confidence value; noul answers have no
separate confidence because the probability already is one.

## Host Implementation

`everruns-host` owns the concrete service behind its optional
`typesafe-judgment` feature ([`crates/host/src/judgment.rs`](../../crates/host/src/judgment.rs)),
backed by the standalone [`typesafe-systemone`](../../crates/drivers/typesafe/README.md)
client. Nothing above core learns the vendor.

That client is filed under `crates/drivers/` as a **judgment driver**: the same
shape as the LLM wire-protocol drivers — a vendor client below the platform
layer, so host and everything above it can depend on it without a cycle — but
it answers typed questions rather than chat completions, so it is not
registered in `DriverRegistry`. It is also the reason the client is a separate
crate from `integrations/typesafe`: an integration crate depends on
`everruns-platform`, which depends on `everruns-host`, so a single crate
carrying both the client and the connector would close that loop.

- Model is fixed (`jev-latest`), for the same reason the utility model is: call
  sites must not be able to turn it into a selectable one.
- Configured from process environment: `TYPESAFE_API_KEY`. Unset or empty means
  the service is disabled and `is_configured()` is false.
- Credentials are deployment-owned. The agent-facing `typesafe` capability is a
  separate surface with its own user-scoped connection; the two never share a
  key (THREAT[TM-LLM-021]).
- Transport is host-owned. Like the utility LLM service, it does not route
  through `EgressService` and is not governed by tenant/agent egress policy.

## Callers

`guardrails` is the first caller: `llm_judge` and `moderation` checks set
`engine: "judgment"` to be answered here instead of by the utility model. See
[Guardrails](../execution/guardrails.md#check-engines).

Every caller must treat an unconfigured or failing service the way the utility
model's callers do — degrade, never wedge a turn. For guardrails that means
fail open, and it is why `is_configured()` exists.

## Data Egress

The state a caller sends is the content being judged: tool arguments, tool
results, or finalized assistant text. It leaves the platform for the judgment
provider, exactly as the moderation path already does for the utility model
provider. Callers bound what they send; guardrails caps stage content at 2 KiB
(tool seams) and 4 KiB (output).
