---
type: Specification
title: "Decision Service"
description: "Internal typed-decisions for capability internals."
tags:
  - everruns
  - operations
---
# Decision Service

<!-- Design Decisions:
  - Modeled on the utility LLM service rather than as a model provider: same
    host-owned, deployment-configured, never-agent-configurable posture. The
    two are siblings, not layers.
  - The contract is provider-neutral (noul/choice/score), not TypeSafe-shaped.
    Core names none of the vendor; the integration crate owns the adapter.
    Swapping vendors, or classifying with a fine-tuned local model, changes
    only what a composition passes in.
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

A decisions removes that class outright, and adds two things a chat
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

`everruns-core` owns the abstraction ([`crates/core/src/decisions.rs`](../../crates/core/src/decisions.rs)):

- `DecisionsService` is the async trait used by capability internals.
- `DecisionQuestion` is one of three primitives — `Noul` (probability of yes),
  `Choice` (one option plus its distribution), `Score` (a position across
  ordered levels plus its distribution).
- `DecisionRequest` carries the state, an ordered list of `(id, question)`,
  an optional model name, and attribution metadata. Ids are for the caller's
  code and never reach the model, so every question must carry its full
  meaning.
- `DecisionAnswer` exposes `probability_yes`, `confidence`, and
  `probability_at_or_above` — the last is the honest reading for a
  "did anything serious happen" rule.
- `DecisionsService::is_configured()` reports whether the deployment enabled it.
- `HostComposition` carries the active service; `ToolContext` and
  `PostGenerationOutputContext` thread it to capability hooks, alongside the
  utility LLM service.

A noul near 0.5 means yes and no are near-equally likely. It does not mean
"medium intensity", and it is not a confidence value; noul answers have no
separate confidence because the probability already is one.

## Implementation

[`integrations/typesafe`](../../integrations/typesafe/README.md) owns the
concrete service ([`src/decisions.rs`](../../integrations/typesafe/src/decisions.rs))
and the vendor client it calls ([`src/client`](../../integrations/typesafe/src/client/)).
Nothing above core learns the vendor.

**One crate, composed from above.** The earlier split — vendor client under
`crates/drivers/`, capability under `integrations/` — existed because
`everruns-host` held the service, and a host dependency cannot point at an
integration crate (integration → `everruns-platform` → `everruns-host` would
close the loop). Moving the service into the integration crate removes the
constraint instead of working around it: `crates/server` and `crates/worker`
already depend on integrations, so they compose the service into
`HostComposition` from above, and the client needs only one home. Host no
longer knows TypeSafe exists.

The crate is published, so the `everruns` facade re-exports `TypeSafeAI` and
`Jev` behind its `typesafe` feature: an embedding application reaches both
halves through one import, exactly as it does for OpenAI.

**Named by layer.** The vendor's own concept model has three
([docs.typesafe.ai](https://docs.typesafe.ai/concepts/system-one)): TypeSafe is
the company and the account that issues the key; **System One is a class of
model**, not a product — models built to return typed decisions and calibrated
probabilities rather than text, the way "LLM" names a class; and Jev is
TypeSafe's flagship model and the first System One model. `DecisionsService` is
this repo's vendor-neutral name for that class, which is why its primitives are
System One's three and why core can name no vendor.

So the provider type is `TypeSafeAI`, mirroring `OpenAI` — transport and
credentials, named for the account. It carries the company's full name because
the short one collides with the host language: in Rust, `TypeSafe` reads as a
marker about type safety and `TypeSafeClient` as "a type-safe client". Only type
names moved; the `TYPESAFE_API_KEY` variables, the `typesafe` feature and crate,
and the stored `typesafe` connection provider are not type positions. The model
is a string id (`jev-latest`, `jev-1.13.0`), the way `gpt-5.6-terra` is. Jev
gets no type because Jev is a model. What an agent sees stays model-named, since a model
is what answers it: the `Jev` capability, the `jev_decision` tool, and the `jev`
guardrail engine.

- The model is named, not defaulted. `Decisions::new` takes it up front the
  way `Model::new` does, because the service is transport and the model is what
  answers; `DecisionRequest::model` overrides it for one call. This is the
  difference from the utility LLM service, whose model is fixed: there will be
  other decision services and other versions of this one, and a threshold calibrated
  against one version is not evidence about the next, so inheriting a vendor
  default silently is the wrong default. The service keeps its own default for
  the wire contract — `DecisionRequest::model` stays optional — which is
  how the platform composes a request that names no model: the knob is absent
  from the guardrail config an agent author writes, rather than absent from the
  type (THREAT[TM-LLM-037]).
- Ids pass through verbatim; nothing here rewrites them, matching the catalog
  contract that ids are the provider's own
  ([`crates/everruns/src/models.rs`](../../crates/everruns/src/models.rs)). The
  vendor accepts `jev-latest` and exact versions, and rejects bare `jev` with
  `Unknown model`. Mapping `jev` to `jev-latest` here would invent an id the
  vendor does not know, so a value copied out of our docs into the vendor's own
  API would fail, and an answer would report a version for an id never sent.
- Two credentials, two audiences: `SystemDecisionsConfig::from_env` reads the
  platform's `UTILITY_TYPESAFE_API_KEY`, while `TypeSafeAI::from_env`
  reads an embedding application's own `TYPESAFE_API_KEY` — the latter is what
  [`Decisions`](../framework/application-api.md#direct-decision-boundary) uses outside the platform.
- Configured from process environment: `UTILITY_TYPESAFE_API_KEY`. Unset or
  empty means the service is disabled and `is_configured()` is false. The name
  mirrors `UTILITY_OPENAI_API_KEY`: both are platform-owned credentials for
  internal model work, distinct from the `TYPESAFE_API_KEY` session secret the
  agent-facing capability falls back to.
- Credentials are deployment-owned. The agent-facing `jev` capability is a
  separate surface with its own user-scoped connection; the two never share a
  key (THREAT[TM-LLM-021]).
- Transport is host-owned. Like the utility LLM service, it does not route
  through `EgressService` and is not governed by tenant/agent egress policy.

## Callers

`guardrails` is the first caller: `llm_judge` and `moderation` checks set
`engine: "jev"` to be answered here instead of by the utility model. The
config value names the model rather than this service, because that is the
choice an agent author is making. See
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
