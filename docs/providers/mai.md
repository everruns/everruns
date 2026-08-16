---
title: Microsoft MAI
description: Run Everruns agents on Microsoft MAI models served via Azure AI Foundry, authenticated with an API key or Microsoft Entra ID (OAuth).
sidebar:
  label: Microsoft MAI
---

<svg role="img" aria-label="Microsoft MAI logo" width="56" height="56" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg" style="float: right; margin-left: 16px;"><path d="M2 2h9.2v9.2H2zM12.8 2H22v9.2h-9.2zM2 12.8h9.2V22H2zM12.8 12.8H22V22h-9.2z"/></svg>

Everruns runs agents on Microsoft MAI models (for example `MAI-Code-1-Flash`),
which are served via [Azure AI Foundry](https://ai.azure.com) behind an
OpenAI-compatible Chat Completions API. The MAI provider exists as its own driver
mainly because of its authentication options.

## What you get

- **OpenAI-compatible Chat Completions** streaming through Azure AI Foundry.
- **Two authentication schemes**: an Azure AI Foundry API key, or Microsoft
  Entra ID (OAuth) service-principal credentials with bearer tokens minted and
  refreshed automatically.
- **Model discovery** against Foundry's `/models` endpoint where available, with
  capabilities supplied by Everruns' built-in Microsoft model profiles.

## Configure in Everruns

1. Go to **Settings** → **Providers** and click **Add provider**.
2. Choose **Microsoft MAI**.
3. Set the **base URL** to your Azure AI Foundry resource
   (e.g. `https://<resource>.services.ai.azure.com`).
4. Provide credentials for one of the two methods, entered as discrete fields:
   - **API key**: your Azure AI Foundry resource key.
   - **Entra ID (OAuth)**: a client-credentials service principal: `tenant_id`,
     `client_id`, and `client_secret` (with optional `scope` and `authority`,
     which default to the Azure Cognitive Services scope and public Microsoft
     Entra authority).
5. Save.

Authentication is fail-closed: a stored credential is always required, and OAuth
tokens are refreshed transparently for both chat execution and model sync.

## Models

Foundry's `/models` listing is bare (ids only), so capabilities come from
Everruns' built-in Microsoft MAI model profiles, matched by id. Because Azure
deployment names are operator-chosen, a deployment whose name does not match a
known profile falls back to a minimal profile.

## Links

- [Azure AI Foundry](https://ai.azure.com/)
- [`everruns-mai` on crates.io](https://crates.io/crates/everruns-mai)
- [Migrate between providers](/how-to/migrate-providers/)
