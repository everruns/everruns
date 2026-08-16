---
title: Anthropic
description: Run Everruns agents on Anthropic Claude models via the Messages API, with streaming, tool use, and extended thinking.
sidebar:
  label: Anthropic
---

<svg role="img" aria-label="Anthropic logo" width="56" height="56" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg" style="float: right; margin-left: 16px;"><path d="M17.304 3h-3.437l5.73 18h3.437L17.304 3zM6.696 3l-5.73 18H4.43l1.307-4.26h5.905L12.95 21h3.466L10.696 3H6.696zm.64 10.74L9.2 7.895l1.864 5.845H7.336z"/></svg>

Everruns runs agents on [Anthropic](https://www.anthropic.com/) Claude models
through the Claude Messages API, mapping its provider-neutral messages, tools,
and reasoning onto the Anthropic wire format.

## What you get

- **Claude Messages API** streaming.
- **Tool use** mapped to provider-neutral Everruns tools.
- **Extended thinking**: adaptive thinking on recent Claude families and
  budget-based thinking on older ones, with the chain-of-thought signature
  preserved across multi-turn conversations.
- **Prompt caching** via bounded `cache_control` breakpoints on stable, high-value
  sections of the request.

## Configure in Everruns

1. Go to **Settings** → **Providers** and click **Add provider**.
2. Choose **Anthropic**.
3. Paste your Anthropic API key. Get one from the
   [Anthropic Console](https://console.anthropic.com/).
4. Save. Everruns discovers available Claude models automatically.

## Models

Anthropic's `/v1/models` endpoint returns capability metadata, which Everruns
merges with its built-in model profiles. Hardcoded profiles take precedence for
cost data; discovered data fills gaps for newer models.

`max_tokens` is required on every Anthropic request, so Everruns always resolves
a value from the model profile (falling back to a safe default) and will retry
once with a lower limit if a stale profile causes the provider to reject it.

## Links

- [Anthropic](https://www.anthropic.com/)
- [Anthropic Console](https://console.anthropic.com/)
- [`everruns-anthropic` on crates.io](https://crates.io/crates/everruns-anthropic)
- [Migrate between providers](/how-to/migrate-providers/)
