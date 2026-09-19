---
title: Vercel AI Gateway
description: Run Everruns agents through Vercel AI Gateway, reaching many upstream model providers over the Open Responses API with one key and automatic model discovery.
sidebar:
  label: Vercel AI Gateway
---

<svg role="img" aria-label="Vercel logo" width="56" height="56" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg" style="float: right; margin-left: 16px;"><path d="M12 3.5 22 20.5H2L12 3.5z"/></svg>

Everruns runs agents through
[Vercel AI Gateway](https://vercel.com/docs/ai-gateway), which fronts many
upstream model providers behind one endpoint. The gateway serves the
[Open Responses](https://openresponses.org) specification, the same
vendor-neutral standard Everruns prefers for new drivers, so this provider gets
the richer surface rather than Chat Completions.

## What you get

- **Many vendors, one key**: OpenAI, Anthropic, Google, and the rest of the
  gateway's catalog from a single Everruns provider.
- **Open Responses**: semantic streaming events, tool calling, structured
  output, and reasoning control over a spec Everruns implements natively.
- **Automatic model discovery**: the gateway's `/models` endpoint is synced, so
  the catalog appears in the agent and session model pickers.
- **Host-gated discovery**: model sync runs only against Vercel's own host, so a
  custom proxy base URL is never probed.

## Configure in Everruns

1. Create an AI Gateway API key in the
   [Vercel dashboard](https://vercel.com/d?to=%2F%5Bteam%5D%2F%7E%2Fai%2Fapi-keys).
2. Go to **Settings** → **Providers** and click **Add provider**.
3. Choose **Vercel AI Gateway**.
4. Paste the key. A Vercel OIDC token works in the same field — the gateway
   accepts either as a bearer token.
5. Save. Everruns discovers the available models automatically.

You can optionally set a base URL to route through a proxy; leave it blank to
use the hosted gateway (`https://ai-gateway.vercel.sh/v1`).

## Models

Model ids are namespaced by upstream provider — `anthropic/claude-opus-5`,
`openai/gpt-6-astra` — and are passed through to the gateway unchanged. After a
sync they appear in the model pickers; Everruns matches a namespaced id against
its model profile registry for capability and cost metadata.

## Links

- [Vercel AI Gateway](https://vercel.com/docs/ai-gateway)
- [Open Responses on AI Gateway](https://vercel.com/docs/ai-gateway/sdks-and-apis/openresponses)
- [`everruns-drivers` on crates.io](https://crates.io/crates/everruns-drivers)
- [Migrate between providers](/how-to/migrate-providers/)
