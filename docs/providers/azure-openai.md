---
title: Azure OpenAI
description: Run Everruns agents on OpenAI models deployed in Azure OpenAI, using a dedicated provider type with your resource endpoint and key.
sidebar:
  label: Azure OpenAI
---

[Azure OpenAI](https://azure.microsoft.com/products/ai-services/openai-service)
serves OpenAI models from your own Azure resource. Everruns ships a dedicated
`azure_openai` provider type — distinct from the [OpenAI](/providers/openai/)
provider — so Azure deployments resolve with the right endpoint and model
behavior rather than being configured as a generic OpenAI base-URL override.

## What you get

- **Azure OpenAI Responses API** through your Azure resource endpoint.
- **Stateful continuation and context compaction** — Azure OpenAI hosts are
  recognized as stateful, like `api.openai.com`.
- **Streaming, tool calls, and reasoning** mapped to provider-neutral Everruns
  types.

## Configure in Everruns

1. Go to **Settings** → **Providers** and click **Add provider**.
2. Choose **Azure OpenAI** (not plain OpenAI).
3. Set the **base URL** to your Azure OpenAI resource endpoint.
4. Paste the API key for your Azure OpenAI resource.
5. Save. Note that Azure model availability depends on the deployments you have
   created in your resource.

## Models

Azure deployment names are operator-chosen, so a deployment whose name does not
match a known model profile falls back to a minimal profile. Capability and cost
metadata for recognized models come from Everruns' built-in model profiles.

## Links

- [Azure OpenAI Service](https://azure.microsoft.com/products/ai-services/openai-service)
- [Azure AI Foundry](https://ai.azure.com/)
- [Migrate between providers](/how-to/migrate-providers/)
