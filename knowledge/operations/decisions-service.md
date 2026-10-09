---
type: Specification
title: "Decision Service"
description: "Typed judgments through authenticated provider accounts, with separate deployment and tenant authority."
tags:
  - everruns
  - operations
---
# Decision Service

## Intent

Typed judgments return calibrated probabilities, selections and scores without parsing prose.
Provider accounts own authentication once; credential-free decision drivers implement the protocol
alongside chat and embedding drivers. Model namespaces never select accounts. The explicit provider
and model pair follows the same selection contract as chat.

## Decision drivers

[Neutral SPI](../../crates/contracts/src/decision_driver.rs) and
[request/outcome contracts](../../crates/contracts/src/decisions.rs) live below concrete vendors.
[Runtime providers](../../crates/contracts/src/runtime_provider.rs) own endpoint and refreshable
authentication shared by their typed services. [System One](../../crates/drivers/drivers/src/systemone/mod.rs)
is shared by direct TypeSafe and OpenRouter. Its response validator rejects missing answers, type
mismatches, invalid distributions and out-of-range scores. Exact provider aliases are explicit;
unknown IDs stay opaque. Requested and returned snapshot identities remain separate.

The [host router](../../crates/core/src/host/decisions/registry.rs) composes a provider registry and
an explicit default model. The utility LLM fallback remains opt-in and reports uncalibrated labels.
The [OpenAI driver](../../crates/drivers/drivers/src/openai/decisions/mod.rs) follows the published
Decisions API (public beta): every primitive is native, all questions go out in one call under
positional names, and a refusal fails the request. The deployment's OpenAI utility key makes it
available; `UTILITY_DECISION_DRIVER=openai` makes it the default. Tenants get it
as the `gpt-6-luna-decisions` catalog model on an OpenAI provider: a distinct id because a provider
holds each model id once and `gpt-6-luna` is already the chat row; the driver sends it as `gpt-6-luna`.
A [live smoke](../../crates/integrations/tests/openai_decisions_live.rs) runs on every main push
that touches it and in the weekly live sweep.

[Microsoft Foundry](../../crates/drivers/drivers/src/mai/decisions.rs) serves Microsoft-Decision-1
over System One at the resource root (`/providers/microsoft/v1/systemone`), on the same MAI
provider, key and base URL as chat. Foundry routes by deployment name, so the catalog row's model
id is the deployment and is sent unchanged; `microsoft-decision-1` and the portal's short
`decision-1` bind the curated profile. Tenant rows authenticate with the resource API key: the
binding carries one key, so Entra-only providers fail closed. Its
[live smoke](../../crates/integrations/tests/foundry_decisions_live.rs) runs the same way.

## Deployment authority

[System configuration](../../crates/worker/src/system_decisions.rs) composes server and worker guardrails
from utility credentials. Guardrail authors cannot select another account or model. With no configured
default, the service is disabled. Guardrails keep their established fail-open behavior on unavailable
or failing utility services. Tenant model defaults affect these calls only through the org setting
below.

## Organization choice for deployment-owned checks

An org's `system_decisions` setting ([migration](../../crates/server/migrations/185_org_system_decisions.sql))
picks who answers guardrail `jev` checks and the Slack relevance check. `deployment` (the default)
keeps the deployment service above. `organization` sends them to the org's decision default on its own
account. The [session decisions wrapper](../../crates/core/src/system_decisions.rs) resolves
the choice per check through the session's credential store and runs an org model through the host's
[bound executor](../../crates/integrations/src/typesafe/bound.rs), the Jev tool's egress, budget and
usage path. An org that opted in with no usable model gets an error: guardrails fail open and Slack
stays silent. Nothing falls back to deployment keys.

[Slack](../../crates/server/src/channels/slack/events/org_decisions.rs) decides a message in a thread that
has a session on that session's budget and ledger. A message that would start a session runs
session-less: personal providers fail closed, the call uses host runtime egress with DNS pinning, no
budget is checked, and usage goes to a structured log line instead of a session ledger.

## Tenant authority

The [Jev capability](../../crates/integrations/src/typesafe/capability.rs) may bind a saved decision model or
use the organization's explicit decision default. Selection refers to an exact model row, which owns
its provider account, wire ID and stable profile binding. Missing, disabled, wrong-service, unhealthy
or foreign selections fail closed; they never fall through to another account or deployment keys.
Legacy TypeSafe connections/session secrets remain available when no catalog selection exists.

[Host resolution](../../crates/server/src/services/provider_resolver/mod.rs) authorizes the session and
account per call, identically through direct and worker adapters. [Bound execution](../../crates/integrations/src/typesafe/bound.rs)
uses session egress policy and DNS pinning, checks budgets before transport, validates outcomes, and
emits usage through the existing generation ledger. Provider-reported cost wins over profile estimates.
No retries duplicate a billable call. TypeSafe, OpenRouter and Foundry rows speak System One, OpenAI rows the
Decisions API; both share that one egress, budget and usage path. State and secrets are absent from usage metadata.

## Catalog and UI

[Catalog assignment](../../crates/server/src/domains/models/catalog.rs) persists service and profile
identity at writes and discovery. Migration backfills existing rows once. Preference edits and sync
preserve the binding. Curated profiles are read-only; discovered/custom profiles remain scoped to their
catalog account. Provider capability masks still apply to chat profile reads.

[Model APIs](../../crates/server/src/api/models.rs) expose service filters, profiles and the decision
default under the existing model permissions. Chat defaults reject non-chat models. The UI offers
service filters, read-only profile inspection and a [decision picker](../../apps/ui/src/components/models/model-picker.tsx)
that retains invalid saved selections for repair. [Capability configuration](../../apps/ui/src/components/agents/capability-settings-editor.tsx)
uses that picker; tool arguments cannot override the account or model.

## Framework and ownership

[Framework Jev](../../crates/integrations/src/typesafe/framework.rs) accepts an authenticated runtime provider
and model pair; existing key/client conveniences remain. [Decisions](../../crates/everruns/src/decisions.rs)
uses the same registry. EVE-1155 ownership is preserved without adding a published crate: contracts
own the SPI, drivers own protocols, integrations own capability/connector adapters, core owns host
composition, server owns tenant persistence/authorization, and worker owns remote execution.

The [design proposal](openrouter-decisions-proposal.md) records alternatives and release boundaries.
