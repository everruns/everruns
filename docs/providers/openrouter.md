---
title: OpenRouter
description: Run Everruns agents across OpenRouter's multi-vendor model catalog with one key, plus provider routing, fallbacks, and capacity controls.
sidebar:
  label: OpenRouter
---

Everruns runs agents on [OpenRouter](https://openrouter.ai/)'s model catalog
through its OpenAI-compatible Responses API. One OpenRouter key gives you access
to a large multi-vendor catalog, plus routing controls that decide which upstream
provider actually serves each request.

## What you get

- **One key, many models** — a single provider exposing OpenRouter's full catalog.
- **Provider routing** — order, allow/deny lists, data-retention and
  zero-data-retention policies, and price/throughput sorting.
- **Capacity strategy** — use OpenRouter's shared capacity, prefer your own
  bring-your-own-key (BYOK) providers first, or require BYOK-only routing.
- **Routing presets** — high-level intents such as cheapest-with-tools,
  lowest-latency, or reasoning-required that compile into the underlying routing
  flags.
- **Capability profiling** — OpenRouter's richer `/models` metadata is parsed into
  capability profiles, so reasoning support surfaces correctly even for models
  without a built-in profile.
- **Session grouping** — Everruns forwards its session id so all generations from
  one session group together in the OpenRouter dashboard.

## Configure in Everruns

1. Go to **Settings** → **Providers** and click **Add provider**.
2. Choose **OpenRouter**.
3. Paste your OpenRouter API key. Get one from the
   [OpenRouter keys page](https://openrouter.ai/keys).
4. Save. Everruns discovers available models and their capability profiles
   automatically.

## Routing controls

OpenRouter-specific routing (model fallbacks, provider ordering, capacity
strategy, and presets) is configured per agent and applied only to OpenRouter
requests — direct OpenAI or other providers ignore these extensions. BYOK-only
routing requires you to list at least one upstream provider, and fails closed if
none is configured.

## Links

- [OpenRouter](https://openrouter.ai/)
- [OpenRouter docs](https://openrouter.ai/docs)
- [`everruns-openrouter` on crates.io](https://crates.io/crates/everruns-openrouter)
- [Migrate between providers](/how-to/migrate-providers/)
