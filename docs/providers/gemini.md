---
title: Google Gemini
description: Run Everruns agents on Google Gemini models, with streaming, tool calls, reasoning, and context caching.
sidebar:
  label: Google Gemini
---

<svg role="img" aria-label="Google Gemini logo" width="56" height="56" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg" style="float: right; margin-left: 16px;"><path d="M12 0C12 6.627 6.627 12 0 12c6.627 0 12 5.373 12 12 0-6.627 5.373-12 12-12-6.627 0-12-5.373-12-12z"/></svg>

Everruns runs agents on [Google Gemini](https://ai.google.dev/) models,
implementing the provider-neutral driver contract over the Gemini API.

## What you get

- **Gemini API** streaming.
- **Tool calls and reasoning** mapped to provider-neutral Everruns types.
- **Context caching**: explicit caching via `cachedContent` when a cached-content
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
