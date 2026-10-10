---
type: Decision
title: "Models and Providers"
description: "Why providers live under Registries → Models with Models, Providers and Defaults tabs, share the models permission, and how new and stale models are surfaced."
tags:
  - everruns
  - ui
  - models
  - providers
---

# Models and Providers

## Purpose

One page for everything that decides which models agents can use: the
models page (`apps/ui/src/app/(main)/models/page.tsx`) under Registries,
with three tabs. **Models** lists every model from every provider, **Providers**
lists the connected vendor accounts, **Defaults** holds the org-wide picks per
service. The provider record itself is described in
[Providers](../foundations/providers.md).

## Decisions

- **Providers moved out of Settings.** Connecting a provider, syncing it and
  choosing its models is one task, and a provider is something an org builds
  and maintains like an agent or MCP server, not a workspace preference.
  `/settings/providers` and `/settings/providers/{id}` redirect to
  `/models?tab=providers` and `/models/providers/{id}` so old links keep working.
- **One permission for both.** Providers and their models are one registry, so
  `provider.*` and `model.*` policies are both gated by `org:models:view` and
  `org:models:manage` ([permissions](../../crates/core/src/permissions.rs)).
  Only holders of the manage permission (org admins by default) connect, rotate,
  sync or delete providers and enable models; everyone else sees the page
  read-only. The API paths did not change.
- **Several providers of one driver, unique names.** Two Azure resources or a
  prod and a dev OpenAI key are normal, so the driver is not a key. The name is:
  it is unique per org, compared without case or surrounding spaces, and a
  duplicate is a `409 provider_name_taken`. Model pickers show
  `Model (Provider name)` because two providers can serve the same model id.
  A personal ChatGPT provider owned by someone else does not reserve a name.
- **Connect ends in choosing models.** Creating a provider with a credential
  discovers its catalog before the request returns, so the connect sheet's last
  step is a model picker with the recommended ones ticked: the newest release of
  each model family per service, plus calibrated decision models. Uncurated
  models are never pre-ticked, which keeps a several-hundred-model catalog such
  as OpenRouter down to a short list.
- **New models are tracked, stale ones are marked, never deleted.** A provider
  keeps `models_reviewed_at`; a discovered, not yet enabled model created after
  it is `is_new` and surfaces as "N new models · Review". Choosing or reviewing a
  provider's models stamps it (`POST /v1/providers/{id}/models/review`). A
  discovered model the provider stopped listing in its last sync is `stale`:
  agents and defaults that point at it keep their reference, and the UI says it
  is no longer listed instead of the row vanishing.
- **Built for scale.** Enabled models are the default view once any exist, rows
  render 50 at a time, and bulk changes go through the selection drawer rather
  than row by row. Bulk enable is a batch of per-model updates from the client;
  there is no bulk endpoint.
- **Defaults say when they cannot be used.** Pickers are searchable. A default
  that is missing, disabled, stale or on a provider without a key gets a warning
  on its row. The decision default accepts calibrated models only. When system
  decisions use the org's own decision model there is no fallback to deployment
  keys, and the row says so.
- **Managed and personal providers.** Host-managed providers keep their
  credential and catalog with the host, so credentials are read-only; choosing
  which of their models to enable stays with the org. A personal ChatGPT provider
  is visible only to its owner and is skipped by shared defaults.

## Limits

- Stale-model warnings cover org defaults, not agents bound to a stale model.
