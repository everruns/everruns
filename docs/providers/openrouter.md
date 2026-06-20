---
title: OpenRouter
description: Run Everruns agents across OpenRouter's multi-vendor model catalog with one key, plus provider routing, fallbacks, and capacity controls.
sidebar:
  label: OpenRouter
---

<svg role="img" aria-label="OpenRouter logo" width="56" height="56" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg" style="float: right; margin-left: 16px;"><path fill-rule="evenodd" d="M16.804 1.957l7.22 4.105v.087L16.73 10.21l.017-2.117-.821-.03c-1.059-.028-1.611.002-2.268.11-1.064.175-2.038.577-3.147 1.352L8.345 11.03c-.284.195-.495.336-.68.455l-.515.322-.397.234.385.23.53.338c.476.314 1.17.796 2.701 1.866 1.11.775 2.083 1.177 3.147 1.352l.3.045c.694.091 1.375.094 2.825.033l.022-2.159 7.22 4.105v.087L16.589 22l.014-1.862-.635.022c-1.386.042-2.137.002-3.138-.162-1.694-.28-3.26-.926-4.881-2.059l-2.158-1.5a21.997 21.997 0 00-.755-.498l-.467-.28a55.927 55.927 0 00-.76-.43C2.908 14.73.563 14.116 0 14.116V9.888l.14.004c.564-.007 2.91-.622 3.809-1.124l1.016-.58.438-.274c.428-.28 1.072-.726 2.686-1.853 1.621-1.133 3.186-1.78 4.881-2.059 1.152-.19 1.974-.213 3.814-.138l.02-1.907z"/></svg>

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
