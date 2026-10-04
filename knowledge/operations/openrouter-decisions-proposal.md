---
type: Proposal
title: "Jev decisions through OpenRouter"
description: "Let one authenticated provider expose chat, decisions, and embeddings services, with Jev served through TypeSafe or OpenRouter."
tags:
  - everruns
  - operations
  - decisions
  - openrouter
  - typesafe
---
# Jev decisions through OpenRouter

Status: accepted and implemented for the first release. The authoritative execution contract is
[Decision Service](decisions-service.md). This proposal records the alternatives and scope decision.

## Problem

The original decision registry selected drivers from model-name prefixes and let each driver own
credentials. That differed from chat's explicit provider/model pair and required a second credential
entry when one account offered several model services. Adding another prefix for OpenRouter would
have preserved the ambiguity and duplicated account management.

## Alternatives

| Option | Benefit | Cost |
| --- | --- | --- |
| Add an OpenRouter-specific decision registry entry | Small initial transport change | Another credential owner and prefix-routing convention |
| Use generic chat structured output | Reuses existing chat calls | Cannot claim System One calibration or measured distributions |
| Extend authenticated runtime providers with typed services | Reuses account identity, endpoint, authentication and model selection | Requires contracts, catalog and consumer changes together |

The third option is selected. A provider account owns authentication; protocol drivers implement
chat, decisions or embeddings against that account. A model name remains opaque after provider
selection. Explicit protocol aliases may translate request IDs within one selected provider without
changing the serving account.

## Ownership and EVE-1155

Use the consolidated crate layout; add no published package. Neutral service contracts belong in
[contracts](../../crates/contracts/src/decision_driver.rs), vendor protocols in the existing
[drivers](../../crates/drivers/drivers/src/systemone/mod.rs), and host composition in
[core](../../crates/core/src/host/decisions/registry.rs). Integrations retain capabilities,
connectors and application conveniences. Server owns tenant catalog, credential resolution and
policy; worker carries those bindings through the existing internal RPC boundary.

This keeps the same serving provider available for several services while leaving each service's
request, response and validation semantics distinct. It also leaves refreshable authentication at
one boundary rather than copying a key into each protocol implementation.

## Product scope

The first release includes saved decision models, stable profile assignment, service-aware APIs and
selectors, an organization decision default, and exact model selection in the Jev capability.
Curated Jev profiles are shared across verified direct and gateway offerings. Moving aliases retain
their identity; execution records the actual returned snapshot separately.

Reuse the existing Models experience and provider accounts. Profiles describe model behavior;
provider-specific model entries describe account availability, wire ID and preferences. Service
support comes from driver declarations. Decision models must not appear in chat or embedding
selectors. An invalid saved selection stays visible with repair guidance and fails closed at runtime.

The existing TypeSafe connection and application client conveniences remain usable when no saved
model selection exists. A configured but broken saved selection must never fall through to them.

## Authority and accounting

Deployment utility decisions serve guardrails and retain their separate keys, defaults and opt-in
fallbacks. Tenant decisions resolve only the exact saved provider account; neither tenant defaults
nor tool arguments can spend a deployment key.

Tenant execution uses the established session egress and budget boundaries, validates calibrated
answers, bounds provider responses and emits usage into the existing ledger. Provider-reported
cost takes precedence over profile estimates. Unknown cost stays unknown. Do not retry a billable
System One request automatically.

## Verification

On 2026-10-04 an authenticated OpenRouter System One probe rejected the bare Jev version ID and
accepted its declared gateway alias. A hosted agent then evaluated noul, choice and score together
through a saved account and returned the Jev 1.13 dated snapshot. Its generation event retained the
saved account, requested model, serving snapshot, token counts and actual cost. This verifies the
selected transport and accounting path; it is not a latency or accuracy comparison.

[Manual acceptance case](../test-cases/ui/models/TC003_saved_decision_model.md) covers service
selectors, exact account reuse, unavailable bindings, tenant boundaries and usage attribution.

## Release boundary

Broad vendor inventory and promotion of the provisional OpenAI native Decisions preview remain
outside this release. Their wire behavior and calibration must be verified before advertising them
as tenant decision models. The shared provider seam already supports adding verified services
without another credential registry or published crate.
