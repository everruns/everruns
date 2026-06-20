---
title: Model Providers
description: Connect Everruns to OpenAI, Anthropic, Google Gemini, AWS Bedrock, OpenRouter, and more. Configure provider credentials once and run any agent on any model.
sidebar:
  label: Overview
  order: 0
---

A **provider** is an organization-scoped account on an AI model vendor — OpenAI,
Anthropic, AWS Bedrock, OpenRouter, and others. You configure a provider once
with credentials and connection settings, and it powers the models your agents
run on. Everruns abstracts every vendor behind one uniform driver interface, so
the same agent, prompt, and capabilities run unchanged whether the model is
served by OpenAI, Claude, Gemini, or any OpenAI-compatible endpoint.

## Supported providers

| Provider | Driver | Notes |
|---|---|---|
| [OpenAI](/providers/openai/) | `openai`, `openai_completions` | Responses API (recommended) and Chat Completions. Also drives OpenAI-compatible endpoints via a base URL. |
| [Azure OpenAI](/providers/azure-openai/) | `azure_openai` | OpenAI models deployed in your Azure OpenAI resource. A dedicated provider type. |
| [Anthropic](/providers/anthropic/) | `anthropic` | Claude models via the Messages API, with extended thinking. |
| [Google Gemini](/providers/gemini/) | `gemini` | Gemini models, with implicit and explicit context caching. |
| [AWS Bedrock](/providers/bedrock/) | `bedrock` | Models hosted on Amazon Bedrock via the `ConverseStream` API. |
| [OpenRouter](/providers/openrouter/) | `openrouter` | One key for a large multi-vendor catalog, with provider routing controls. |
| [Microsoft MAI](/providers/mai/) | `mai` | Microsoft MAI models via Azure AI Foundry, with API-key or Entra ID (OAuth) auth. |

Need a vendor that isn't listed? Any OpenAI-compatible endpoint works through the
[OpenAI](/providers/openai/) provider with a custom base URL (use the dedicated
[Azure OpenAI](/providers/azure-openai/) provider for Azure deployments), and
embedders can register additional drivers through the platform definition.

## Configure a provider

Providers are managed by organization admins:

1. Go to **Settings** → **Providers**.
2. Click **Add provider** and choose the provider type.
3. Enter the credentials (API key, or the provider-specific fields described on
   each provider's page). Credentials are validated before they are saved.
4. Save. Everruns discovers the provider's available models and makes them
   selectable for agents and sessions.

You can configure more than one provider for the same vendor — for example two
Azure OpenAI regions, or a direct OpenAI key alongside an OpenRouter key.

## Providers vs. connections

Providers and [connections](/integrations/) look similar but are deliberately
separate:

| | **Provider** | **Connection** |
|---|---|---|
| Scope | Organization | User |
| Purpose | Infrastructure that runs agents | A user's identity on an external service, used by tools |
| Configured in | Settings → Providers | Settings → Connections |
| Examples | OpenAI, Anthropic, Bedrock | Daytona, GitHub, Slack |

Use a provider to decide **which model runs your agents**. Use a connection to
give an agent access to an **external tool or service**.

## Models and switching providers

Agents and sessions bind to a specific model on a specific provider. Because the
driver interface is uniform, you can move an agent from one provider to another
without rewriting prompts or capabilities. See
[Migrate between providers](/how-to/migrate-providers/) for the model-resolution
rules and the API calls to add a provider, switch an agent's default model, or
run an A/B comparison per session.

## Credential security

- Credentials are encrypted at rest with AES-256-GCM envelope encryption.
- Credential values are never returned by the API — only a "configured" flag is
  exposed.
- Resolution is **fail-closed**: tenant agent turns resolve keys from the
  database only. A turn with no configured provider fails with a clear error
  rather than silently falling back to a platform environment variable.

For self-hosted and single-tenant deployments, provider credentials for the
default organization can be seeded from environment variables at startup.
Key-based providers use `DEFAULT_<PROVIDER>_API_KEY` (for example
`DEFAULT_OPENAI_API_KEY`, `DEFAULT_ANTHROPIC_API_KEY`); AWS Bedrock reads
standard `AWS_*` (or `AWS_BEDROCK_*`) credentials instead. See the
[environment variables reference](/sre/environment-variables/) for the exact
names per provider.
