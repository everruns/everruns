---
title: Model Providers
description: Connect Everruns to OpenAI, Anthropic, Google Gemini, Meta Model API, AWS Bedrock, OpenRouter, Fireworks AI, and more. Configure provider credentials once and run any agent on any model.
sidebar:
  label: Overview
  order: 0
---

A **provider** is an organization-scoped account on an AI model vendor, OpenAI,
Anthropic, AWS Bedrock, OpenRouter, and others. You configure a provider once
with credentials and connection settings, and it powers the models your agents
run on. Everruns abstracts every vendor behind one uniform driver interface, so
the same agent, prompt, and capabilities run unchanged whether the model is
served by OpenAI, Claude, Gemini, or any OpenAI-compatible endpoint.

## Supported providers

| Provider | Driver | Notes |
|---|---|---|
| <svg width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg"><path d="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364l2.0201-1.1638a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.4043-.6813zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z"/></svg>[OpenAI](/providers/openai/) | `openai`, `openai_completions` | Responses API (recommended) and Chat Completions. Also drives OpenAI-compatible endpoints via a base URL. |
| <svg width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg"><path d="M13.05 4.24L6.56 18.05L2 18.22L7.68 7.32L13.05 4.24ZM14.15 5.56L16.65 10.25L12.38 18.04L22 18.25L14.15 5.56Z"/></svg>[Azure OpenAI](/providers/azure-openai/) | `azure_openai` | OpenAI models deployed in your Azure OpenAI resource. A dedicated provider type. |
| <svg width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg"><path d="M17.304 3h-3.437l5.73 18h3.437L17.304 3zM6.696 3l-5.73 18H4.43l1.307-4.26h5.905L12.95 21h3.466L10.696 3H6.696zm.64 10.74L9.2 7.895l1.864 5.845H7.336z"/></svg>[Anthropic](/providers/anthropic/) | `anthropic` | Claude models via the Messages API, with extended thinking. |
| <svg width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg"><path d="M12 0C12 6.627 6.627 12 0 12c6.627 0 12 5.373 12 12 0-6.627 5.373-12 12-12-6.627 0-12-5.373-12-12z"/></svg>[Google Gemini](/providers/gemini/) | `gemini` | Gemini models, with implicit and explicit context caching. |
| <svg width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>[AWS Bedrock](/providers/bedrock/) | `bedrock` | Models hosted on Amazon Bedrock via the `ConverseStream` API. |
| <svg width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M11 6l-6 6 6 6"/><path d="M19 6l-6 6 6 6"/></svg>[OpenRouter](/providers/openrouter/) | `openrouter` | One key for a large multi-vendor catalog, with provider routing controls. |
| <svg width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg"><path d="M2 2h9.2v9.2H2zM12.8 2H22v9.2h-9.2zM2 12.8h9.2V22H2zM12.8 12.8H22V22h-9.2z"/></svg>[Microsoft MAI](/providers/mai/) | `mai` | Microsoft MAI models via Azure AI Foundry, with API-key or Entra ID (OAuth) auth. |
| <svg width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" xmlns="http://www.w3.org/2000/svg"><circle cx="12" cy="12" r="1.6" fill="currentColor" stroke="none"/><path d="M12 2.5V6M12 18v3.5M2.5 12H6M18 12h3.5M5.2 5.2l2.5 2.5M16.3 16.3l2.5 2.5M18.8 5.2l-2.5 2.5M7.7 16.3l-2.5 2.5"/></svg>[Fireworks AI](/providers/fireworks/) | `fireworks` | Fast, low-cost inference for open models (Llama, Qwen, DeepSeek, Kimi, GLM, gpt-oss, ...), with automatic model discovery. |
| <svg width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;" viewBox="0 11.34 14.004 9.32" fill="currentColor" xmlns="http://www.w3.org/2000/svg"><path d="M10.0469 11.3486C8.90014 11.3486 8.00191 12.2136 7.18978 13.3056C6.07237 11.8848 5.13892 11.3486 4.02346 11.3486C1.74559 11.3486 0 14.3134 0 17.4504C0 19.4073 0.949114 20.6519 2.54402 20.6519C3.68882 20.6519 4.51269 20.1118 5.97648 17.5521L7.00779 15.7322C7.15456 15.9709 7.30916 16.2253 7.47158 16.5012L8.15847 17.6558C9.49506 19.8926 10.2406 20.6519 11.5909 20.6519C13.1408 20.6519 14.0038 19.3975 14.0038 17.3916C13.9999 14.1079 12.2152 11.3486 10.0469 11.3486ZM4.85712 16.8594C3.66926 18.7204 3.2583 19.1314 2.59881 19.1314C1.93932 19.1314 1.51467 18.5443 1.51467 17.4699C1.51467 15.1921 2.64969 12.8633 4.00389 12.8633C4.73578 12.8633 5.34831 13.286 6.28764 14.6245C5.39723 16.0003 4.85712 16.8594 4.85712 16.8594ZM9.33654 16.6265L8.51463 15.2566C8.2935 14.8946 8.08019 14.5639 7.87471 14.2586C8.61443 13.1177 9.22499 12.5482 9.95102 12.5482C11.4579 12.5482 12.6653 14.7694 12.6653 17.4954C12.6653 18.5345 12.3248 19.1372 11.6183 19.1372C10.9432 19.1314 10.6203 18.6911 9.33654 16.6265Z"/><path d="M8.51465 15.2566C6.7358 12.3623 5.55185 11.3428 4.02348 11.3428L4.00391 12.8633C5.0039 12.8633 5.78081 13.6461 7.46768 16.4954L7.57141 16.6676L8.51465 15.2566Z"/></svg>[Meta Model API](/providers/meta/) | `meta` | Muse Spark through Meta's stateful, OpenAI-compatible Responses API. |

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

You can configure more than one provider for the same vendor, for example two
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
- Credential values are never returned by the API, only a "configured" flag is
  exposed.
- Resolution is **fail-closed**: each organization resolves its own configured
  credentials. A turn with no configured provider fails with a clear error
  rather than running on another organization's credentials.
