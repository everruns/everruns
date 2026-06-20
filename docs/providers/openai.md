---
title: OpenAI
description: Run Everruns agents on OpenAI models via the Responses API or Chat Completions, including Azure OpenAI and any OpenAI-compatible endpoint.
sidebar:
  label: OpenAI
---

Everruns runs agents on [OpenAI](https://openai.com/) models through a uniform
driver, mapping its provider-neutral messages, tools, and reasoning onto the
OpenAI wire format. The same driver also powers any **OpenAI-compatible
endpoint** through a custom base URL. For Azure deployments, use the dedicated
[Azure OpenAI](/providers/azure-openai/) provider instead.

## What you get

- **Responses API** (recommended) — the default driver, with stateful
  continuation and server-side context compaction on `api.openai.com` and Azure
  OpenAI hosts.
- **Chat Completions** — a compatibility driver (`openai_completions`) for legacy
  integrations and OpenAI-compatible gateways.
- **Streaming, tool calls, and reasoning** mapped to provider-neutral Everruns
  types, including extended thinking on reasoning models.
- **Prompt caching** via a deterministic cache key derived from stable request
  inputs.

## Configure in Everruns

1. Go to **Settings** → **Providers** and click **Add provider**.
2. Choose **OpenAI** (Responses API) for new setups, or **OpenAI Completions**
   for Chat Completions compatibility. For Azure, choose the dedicated
   [Azure OpenAI](/providers/azure-openai/) provider instead.
3. Paste your OpenAI API key. Get one from the
   [OpenAI API keys page](https://platform.openai.com/api-keys).
4. (Optional) Set a **base URL** to target another OpenAI-compatible endpoint.
5. Save. Everruns discovers available models automatically.

## OpenAI-compatible endpoints

The OpenAI driver works with any OpenAI-compatible API. Set the **base URL** to
your endpoint. Self-hosted and proxy gateways are treated as stateless, so the
driver replays the full transcript each turn instead of relying on server-side
continuation. For Microsoft Azure deployments, use the dedicated
[Azure OpenAI](/providers/azure-openai/) provider, which is recognized as a
stateful host.

## Models

Models are discovered when the provider is created and on each sync. OpenAI's
`/models` listing carries only identifiers, so capability and cost metadata come
from Everruns' built-in model profiles, matched by model id.

## Links

- [OpenAI Platform](https://platform.openai.com/)
- [`everruns-openai` on crates.io](https://crates.io/crates/everruns-openai)
- [Migrate between providers](/how-to/migrate-providers/)
