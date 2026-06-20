---
title: Google Gemini
description: Run Everruns agents on Google Gemini models, with streaming, tool calls, reasoning, and context caching.
sidebar:
  label: Google Gemini
---

Everruns runs agents on [Google Gemini](https://ai.google.dev/) models,
implementing the provider-neutral driver contract over the Gemini API.

## What you get

- **Gemini API** streaming.
- **Tool calls and reasoning** mapped to provider-neutral Everruns types.
- **Context caching** — explicit caching via `cachedContent` when a cached-content
  resource is supplied, otherwise Gemini's default implicit caching on supported
  models.

## Configure in Everruns

1. Go to **Settings** → **Providers** and click **Add provider**.
2. Choose **Google Gemini**.
3. Paste your Gemini API key. Get one from
   [Google AI Studio](https://aistudio.google.com/apikey).
4. Save. Everruns discovers available Gemini models automatically.

## Models

Gemini models resolve to Everruns' built-in model profiles for capability and
cost metadata. When `max_tokens` is not set, the driver resolves a default from
the model profile, falling back to a safe value.

## Links

- [Google AI for Developers](https://ai.google.dev/)
- [Google AI Studio](https://aistudio.google.com/)
- [`everruns-gemini` on crates.io](https://crates.io/crates/everruns-gemini)
- [Migrate between providers](/how-to/migrate-providers/)
