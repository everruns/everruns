---
title: Cloudflare AI Gateway
description: Run Everruns agents through Cloudflare's AI REST API, reaching many upstream providers and Cloudflare's own Workers AI models with one account token.
sidebar:
  label: Cloudflare AI Gateway
---

<svg role="img" aria-label="Cloudflare logo" width="56" height="56" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg" style="float: right; margin-left: 16px;"><path d="M16.5088 16.8447c.1475-.5068.0908-.9707-.1553-1.3154-.2246-.3164-.6045-.499-1.0615-.5205l-8.6592-.1123a.1559.1559 0 0 1-.1333-.0713c-.0283-.042-.0351-.0986-.021-.1553.0278-.084.1123-.1484.2036-.1562l8.7359-.1123c1.0351-.0489 2.1601-.8868 2.5537-1.9136l.499-1.3013c.0215-.0561.0293-.1128.0147-.168-.5625-2.5463-2.835-4.4453-5.5499-4.4453-2.5039 0-4.6284 1.6177-5.3876 3.8614-.4927-.3658-1.1187-.5625-1.794-.499-1.2026.119-2.1665 1.083-2.2861 2.2856-.0283.31-.0069.6128.0635.894C1.5683 13.171 0 14.7754 0 16.752c0 .1748.0142.3515.0352.5273.0141.083.0844.1475.1689.1475h15.9814c.0909 0 .1758-.0645.2032-.1553l.12-.4268zm2.7568-5.5634c-.0771 0-.1611 0-.2383.0112-.0566 0-.1054.0415-.127.0976l-.3378 1.1744c-.1475.5068-.0918.9707.1543 1.3164.2256.3164.6055.498 1.0625.5195l1.8437.1133c.0557 0 .1055.0263.1329.0703.0283.043.0351.1074.0214.1562-.0283.084-.1132.1485-.204.1553l-1.921.1123c-1.041.0488-2.1582.8867-2.5527 1.914l-.1406.3585c-.0283.0713.0215.1416.0986.1416h6.5977c.0771 0 .1474-.0489.169-.126.1122-.4082.1757-.837.1757-1.2803 0-2.6025-2.125-4.727-4.7344-4.727"/></svg>

Everruns runs agents through
[Cloudflare's AI REST API](https://developers.cloudflare.com/ai-gateway/usage/rest-api/),
which fronts many upstream model providers and Cloudflare's own Workers AI
models behind one account-scoped, OpenAI-compatible endpoint. Every call routes
through an AI Gateway for caching, rate limiting, and logging.

## What you get

- **Many vendors, one token**: OpenAI, Anthropic, Google, xAI and the rest of
  the catalog, plus Workers AI models, from a single Everruns provider.
- **No upstream keys**: Cloudflare authenticates and bills the whole call
  against your account, so there are no per-vendor API keys to manage.
- **Gateway controls**: response caching, rate limiting, retries, and
  per-request logging applied by the gateway the call routes through.
- **Full chat capabilities**: streaming, tool/function calling, and structured
  output, through the same uniform driver as every other provider.

## Configure in Everruns

1. Create an API token with the **Account** → **Workers AI** → **Read**
   permission in the
   [Cloudflare dashboard](https://dash.cloudflare.com/profile/api-tokens).
2. Go to **Settings** → **Providers** and click **Add provider**.
3. Choose **Cloudflare AI Gateway**.
4. Enter:
   - **API Token** — the token from step 1.
   - **Account ID** — the account it belongs to.
   - **Gateway name** — optional. Calls route through the account's default
     gateway when it is unset.
5. Save.

Everruns derives the endpoint from the account id:

```
https://api.cloudflare.com/client/v4/accounts/<account-id>/ai/v1
```

Set a base URL only to route through a proxy in front of Cloudflare; it
replaces the derived one.

## Models

Model ids are namespaced and passed through unchanged:

- Third-party: `openai/gpt-6-luna`, `anthropic/claude-opus-5` — see the
  [upstream providers AI Gateway supports](https://developers.cloudflare.com/ai-gateway/usage/providers/).
- Workers AI: `@cf/meta/llama-3.3-70b-instruct-fp8-fast` — see the
  [Workers AI model catalog](https://developers.cloudflare.com/workers-ai/models/).

Which third-party models a gateway can reach depends on the upstreams
Cloudflare has enabled for your account, so verify an id with a real request
rather than assuming the catalog.

Third-party models are billed to your Cloudflare account and return `402`
("Insufficient balance") until it is funded or you configure BYOK. Workers AI
models draw on the account's own allocation instead.

The AI REST API serves no model catalog, so there is nothing to sync: add the
models you intend to use by id. Everruns matches a namespaced id against its
model profile registry, so a model it recognizes still gets its capability and
cost metadata.

## Why Chat Completions

Cloudflare's REST API offers four formats. Everruns uses
`/ai/v1/chat/completions` because it is the only one that serves the whole
catalog and streams:

| Endpoint | Why not |
| --- | --- |
| `/ai/v1/responses` | Rejects Workers AI models — they are not translated into the Responses shape |
| `/ai/run` | Cannot stream: with `stream: true` it returns an empty JSON body rather than events |
| `/ai/v1/messages` | Excludes Workers AI models |

The older `gateway.ai.cloudflare.com/.../compat/chat/completions` endpoint is
deprecated by Cloudflare for single-model calls and kept only for dynamic
routes (`dynamic/{route}`), which this provider does not target.

## Links

- [Cloudflare AI Gateway](https://developers.cloudflare.com/ai-gateway/)
- [AI Gateway REST API](https://developers.cloudflare.com/ai-gateway/usage/rest-api/)
- [Workers AI models](https://developers.cloudflare.com/workers-ai/models/)
- [AI Gateway providers](https://developers.cloudflare.com/ai-gateway/usage/providers/)
- [`everruns-drivers` on crates.io](https://crates.io/crates/everruns-drivers)
- [Migrate between providers](/how-to/migrate-providers/)
