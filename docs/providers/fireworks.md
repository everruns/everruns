---
title: Fireworks AI
description: Run Everruns agents on Fireworks AI's fast, low-cost inference for open models, Llama, Qwen, DeepSeek, Kimi, GLM, gpt-oss, and more, with automatic model discovery.
sidebar:
  label: Fireworks AI
---

<svg role="img" aria-label="Fireworks AI logo" width="56" height="56" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" xmlns="http://www.w3.org/2000/svg" style="float: right; margin-left: 16px;"><circle cx="12" cy="12" r="1.6" fill="currentColor" stroke="none"/><path d="M12 2.5V6M12 18v3.5M2.5 12H6M18 12h3.5M5.2 5.2l2.5 2.5M16.3 16.3l2.5 2.5M18.8 5.2l-2.5 2.5M7.7 16.3l-2.5 2.5"/></svg>

Everruns runs agents on [Fireworks AI](https://fireworks.ai/) through its
OpenAI-compatible Chat Completions API. Fireworks serves frontier **open
models**: Llama, Qwen, DeepSeek, Kimi, GLM, gpt-oss, and more, on a fast,
cost-efficient inference platform, so the same Everruns agent, prompt, and
capabilities run unchanged on open weights.

## What you get

- **One key, many open models**: a single provider exposing Fireworks' serverless
  model catalog.
- **Automatic model discovery**: Fireworks' `/models` endpoint advertises rich
  metadata (chat, tool calling, image input, context window), which Everruns
  parses into capability profiles on sync, so tool and vision support surface
  correctly per model.
- **Full chat capabilities**: streaming, tool/function calling, vision, and
  structured output, through the same uniform driver as every other provider.
- **Host-gated discovery**: model sync runs only against Fireworks' own host, so
  a custom proxy base URL is never probed.

## Configure in Everruns

1. Go to **Settings** → **Providers** and click **Add provider**.
2. Choose **Fireworks AI**.
3. Paste your Fireworks API key. Create one from the
   [Fireworks API keys page](https://fireworks.ai/account/api-keys).
4. Save. Everruns discovers available models and their capability profiles
   automatically.

You can optionally set a base URL to route through a proxy; leave it blank to use
Fireworks' hosted API (`https://api.fireworks.ai/inference/v1`).

## Models

Fireworks model ids are namespaced, for example
`accounts/fireworks/models/llama-v3p1-70b-instruct`. After a sync, models appear
in the agent and session model pickers with their discovered capabilities. Only
chat models are imported, image and other non-chat endpoints are filtered out.

## Links

- [Fireworks AI](https://fireworks.ai/)
- [Fireworks docs](https://docs.fireworks.ai/)
- [`everruns-fireworks` on crates.io](https://crates.io/crates/everruns-fireworks)
- [Migrate between providers](/how-to/migrate-providers/)
