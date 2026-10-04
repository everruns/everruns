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
  - Vendors are decision drivers behind one router (EVE-1117), the way LLM
    vendors are chat drivers behind a provider. Call sites never learn which
    driver answered; a deployment switches vendors with `DECISIONS_DRIVER`.
  - Calibration is reported, never faked. A label-only driver answers one-hot
    and says `calibrated: false`; it does not invent the numbers in between.
  - The `llm` fallback is opt-in. It spends utility-model tokens on every
    check and answers uncalibrated, so a deployment that never chose it keeps
    today's behavior (TypeSafe with a key, disabled without).
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

- `DecisionOutcome::calibrated` says whether the numbers are a measured
  distribution. When false, the answering driver returned only a label and
  encoded it one-hot, so thresholds read as "did it pick this" and the value
  carries no certainty.

A noul near 0.5 means yes and no are near-equally likely. It does not mean
"medium intensity", and it is not a confidence value; noul answers have no
separate confidence because the probability already is one.

## Decision drivers

A decision driver is one vendor's transport for the contract above, the
decisions equivalent of an LLM chat driver. The trait lives in core
([`crates/core/src/decision_driver.rs`](../../crates/core/src/decision_driver.rs));
the registry and router live in host
([`crates/host/src/decisions/`](../../crates/host/src/decisions/)); vendor drivers
live with their vendor, so host still names none.

- **Capabilities are declared.** A driver states which primitives it answers
  natively, whether its answers are calibrated, whether it takes image state,
  and its limits (questions per request, options per question, state bytes).
  The router rejects a request over a declared limit before any round trip.
  A driver whose vendor lacks a primitive translates it (a noul as a two-way
  choice, a score as a choice over the levels); callers never see a primitive
  refused.
- **Routing by model id.** `driver/model` reaches that driver with the prefix
  stripped; a prefix a driver declares (`jev-` for TypeSafe) reaches it
  untouched; anything else, and a request naming no model, reaches the
  deployment default. An unregistered `x/...` goes to the default unchanged,
  because some vendor ids carry a slash.
- **One service to callers.** The router implements `DecisionsService`, so
  guardrails, the `Decisions` facade, and capability internals are unchanged.
- **Observed per call.** Each evaluation runs in a `decisions.evaluate` span
  carrying the driver id, requested and resolved model, primitive kinds,
  question count, calibration, token usage, and latency. It reaches OTel
  through the tracing bridge, which is how vendors are compared on the
  guardrail path.

Drivers today:

| Driver | Where | Answers | Calibrated |
|---|---|---|---|
| `typesafe` | [`integrations/typesafe`](../../integrations/typesafe/src/decisions.rs) | all three primitives, owns `jev-*` | yes |
| `llm` | [`crates/host/src/decisions/llm.rs`](../../crates/host/src/decisions/llm.rs) | all three, via the utility LLM and a validated JSON reply | no |
| `openai` (preview) | [`integrations/openai-decisions`](../../integrations/openai-decisions/src/lib.rs) | choice native; noul and score asked as choices; one call per question, concurrent | only when a response carries a probability for every label |

The `openai` driver fronts OpenAI's Decisions API (DevDay 2026, limited
preview). Its wire shape is **provisional**: no reference was published, and the
endpoint (`POST /v1/decisions`, confirmed) refused our account with "Decision
API is not enabled for this user" on 2026-09-30. The inferred request and
response live alone in
[`wire.rs`](../../integrations/openai-decisions/src/wire.rs) so verifying them
touches one module and its fixtures. It is registered only with
`DECISIONS_OPENAI_PREVIEW=1`, and the crate is unpublished until the shape is
verified. The single confidence the API is reported to return is not spread
into a distribution. Comparing it with Jev on the guardrail gallery (accuracy,
p50/p95 latency, cost per 1K decisions) waits for access; no default changes
before that.

The `llm` driver answers with the utility model the deployment pinned and
refuses a request naming another model. It sends questions under positional
keys, so caller ids still never reach a model, and rejects any reply that
names an option the question did not offer or a level out of range; callers
fail open on that error exactly as on an outage. It asks for JSON in the
prompt today; once the utility request can carry a structured-output schema
(EVE-1116) it should send one as well.

### Deployment configuration

Composed in [`crates/worker/src/system_decisions.rs`](../../crates/worker/src/system_decisions.rs),
which both the server and the worker platform call, so the two never drift:

- `UTILITY_TYPESAFE_API_KEY` registers `typesafe`.
- A configured utility LLM registers `llm`.
- `DECISIONS_OPENAI_PREVIEW=1` with `UTILITY_OPENAI_API_KEY` registers the
  preview `openai` driver, with retries off like the TypeSafe client.
- `DECISIONS_DRIVER` picks the default driver. Unset: `typesafe` when its key
  is present, otherwise the disabled service.
- `DECISIONS_MODEL` is what the default driver is asked for when a request
  names no model. Refused with `DECISIONS_DRIVER=llm`, whose model is
  `UTILITY_LLM_MODEL`.

A `DECISIONS_DRIVER` naming a driver that is not configured stops startup with
the list of configured drivers and the variables each one needs, rather than
failing open on the first guardrail check.

## TypeSafe driver

[`integrations/typesafe`](../../integrations/typesafe/README.md) owns the
TypeSafe driver ([`src/decisions.rs`](../../integrations/typesafe/src/decisions.rs))
and the vendor client it calls ([`src/client`](../../integrations/typesafe/src/client/)).
Nothing above core learns the vendor.

**One crate, composed from above.** The earlier split — vendor client under
`crates/drivers/`, capability under `integrations/` — existed because
`everruns-host` held the service, and a host dependency cannot point at an
integration crate (integration → `everruns-capabilities` → `everruns-host` would
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
  platform's `UTILITY_TYPESAFE_API_KEY` (and `into_driver` turns it into the
  registered driver), while `TypeSafeAI::from_env`
  reads an embedding application's own `TYPESAFE_API_KEY` — the latter is what
  [`Decisions`](../framework/application-api.md#direct-decision-boundary) uses outside the platform.
- Configured from process environment: `UTILITY_TYPESAFE_API_KEY`. Unset or
  empty means the driver is not registered; with no other driver chosen the
  service is disabled and `is_configured()` is false. The name
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
results, or finalized assistant text. It leaves the platform for the answering
driver's vendor (TypeSafe, or the utility model's provider under `llm`),
exactly as the moderation path already does for the utility model provider. Callers bound what they send; guardrails caps stage content at 2 KiB
(tool seams) and 4 KiB (output).
