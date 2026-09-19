---
title: Cloudflare AI Gateway
description: Run Everruns agents through Cloudflare AI Gateway, reaching many upstream model providers behind one endpoint with Cloudflare's caching, rate limiting, and observability.
sidebar:
  label: Cloudflare AI Gateway
---

<svg role="img" aria-label="Cloudflare logo" width="56" height="56" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg" style="float: right; margin-left: 16px;"><path d="M17.5 19H6.5a4.5 4.5 0 0 1-.36-8.986A6 6 0 0 1 17.83 11.2 4 4 0 0 1 17.5 19z"/></svg>

Everruns runs agents through
[Cloudflare AI Gateway](https://developers.cloudflare.com/ai-gateway/), which
fronts many upstream model providers behind a single OpenAI-compatible
endpoint. Your agents keep one provider configuration while the gateway handles
caching, rate limiting, retries, and request logging.

## What you get

- **Many vendors, one endpoint**: OpenAI, Anthropic, Google, Workers AI, and the
  other upstreams your gateway is configured for, all reachable from one
  Everruns provider.
- **Cloudflare's controls**: response caching, rate limiting, and per-request
  analytics applied before a request ever leaves the gateway.
- **Keys where you want them**: store upstream provider keys in the gateway and
  authenticate only to Cloudflare, or send an upstream key per request.
- **Full chat capabilities**: streaming, tool/function calling, and structured
  output, through the same uniform driver as every other provider.

## Configure in Everruns

1. Create a gateway in the
   [Cloudflare dashboard](https://dash.cloudflare.com/?to=/:account/ai/ai-gateway).
2. Go to **Settings** → **Providers** and click **Add provider**.
3. Choose **Cloudflare AI Gateway**.
4. Enter:
   - **AI Gateway Token** — a Cloudflare API token with the AI Gateway Run
     permission. Everruns sends it as `cf-aig-authorization`.
   - **Account ID** — the Cloudflare account that owns the gateway.
   - **Gateway name** — as it appears in the gateway's URL. Cloudflare's first
     gateway is called `default`.
   - **Upstream provider key** — optional, and only for a gateway that stores no
     provider keys of its own. Everruns sends it as `Authorization`, for
     whichever upstream the model id names.
5. Save.

Everruns derives the endpoint from the account id and gateway name:

```
https://gateway.ai.cloudflare.com/v1/<account-id>/<gateway>/compat
```

Set a base URL only to route through a proxy in front of the gateway; it
replaces the derived one.

## Models

Model ids are namespaced by upstream provider — `openai/gpt-5.2`,
`anthropic/claude-4-5-sonnet`, `workers-ai/@cf/meta/llama-3.3-70b-instruct-fp8-fast`
— and are passed through to Cloudflare unchanged.

The `/compat` surface serves no model catalog, so there is nothing to sync: add
the models you intend to use by id. Everruns matches a namespaced id against its
model profile registry, so a model it recognizes still gets its capability and
cost metadata.

## Links

- [Cloudflare AI Gateway](https://developers.cloudflare.com/ai-gateway/)
- [OpenAI-compatible endpoint](https://developers.cloudflare.com/ai-gateway/usage/chat-completion/)
- [`everruns-drivers` on crates.io](https://crates.io/crates/everruns-drivers)
- [Migrate between providers](/how-to/migrate-providers/)
