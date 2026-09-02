---
title: Meta Model API
description: Run Everruns agents on Muse Spark 1.3 through Meta Model API, including the discounted Contributor tier.
sidebar:
  label: Meta Model API
---

<svg role="img" aria-label="Meta logo" width="56" height="56" viewBox="0 11.34 14.004 9.32" fill="currentColor" xmlns="http://www.w3.org/2000/svg" style="float: right; margin-left: 16px;"><path d="M10.0469 11.3486C8.90014 11.3486 8.00191 12.2136 7.18978 13.3056C6.07237 11.8848 5.13892 11.3486 4.02346 11.3486C1.74559 11.3486 0 14.3134 0 17.4504C0 19.4073 0.949114 20.6519 2.54402 20.6519C3.68882 20.6519 4.51269 20.1118 5.97648 17.5521L7.00779 15.7322C7.15456 15.9709 7.30916 16.2253 7.47158 16.5012L8.15847 17.6558C9.49506 19.8926 10.2406 20.6519 11.5909 20.6519C13.1408 20.6519 14.0038 19.3975 14.0038 17.3916C13.9999 14.1079 12.2152 11.3486 10.0469 11.3486ZM4.85712 16.8594C3.66926 18.7204 3.2583 19.1314 2.59881 19.1314C1.93932 19.1314 1.51467 18.5443 1.51467 17.4699C1.51467 15.1921 2.64969 12.8633 4.00389 12.8633C4.73578 12.8633 5.34831 13.286 6.28764 14.6245C5.39723 16.0003 4.85712 16.8594 4.85712 16.8594ZM9.33654 16.6265L8.51463 15.2566C8.2935 14.8946 8.08019 14.5639 7.87471 14.2586C8.61443 13.1177 9.22499 12.5482 9.95102 12.5482C11.4579 12.5482 12.6653 14.7694 12.6653 17.4954C12.6653 18.5345 12.3248 19.1372 11.6183 19.1372C10.9432 19.1314 10.6203 18.6911 9.33654 16.6265Z"/><path d="M8.51465 15.2566C6.7358 12.3623 5.55185 11.3428 4.02348 11.3428L4.00391 12.8633C5.0039 12.8633 5.78081 13.6461 7.46768 16.4954L7.57141 16.6676L8.51465 15.2566Z"/></svg>

Everruns runs Muse models through [Meta Model API](https://dev.meta.ai/). The
dedicated `meta` driver uses Meta's OpenAI-compatible Responses API at
`https://api.meta.ai/v1`, including streaming, parallel tool calls, reasoning
replay, message phases, hosted tool search, and server-managed response history.

## Configure in Everruns

1. Create an API key in the [Meta Model API dashboard](https://dev.meta.ai/).
2. Go to **Settings** → **Providers** and click **Add provider**.
3. Choose **Meta Model API**, paste the key, and save.
4. Sync models to import the Muse models available to your team.

The hosted endpoint is used by default. An optional base URL can point the
driver at a compatible proxy; model discovery is disabled for non-Meta hosts.

## Muse Spark 1.3 tiers

Muse Spark 1.3 is built for long-horizon coding and multi-step agentic work,
with native tool calling and MCP support. Both 1.3 model IDs have a
1,048,576-token context window and accept text, images, audio, video, and PDFs
while producing text.

| Model | Data use | Input / cached input / output per million tokens |
|---|---|---|
| `muse-spark-1.3` | Prompts and completions are not used to train Meta models | $1.25 / $0.15 / $4.25 |
| `muse-spark-1.3-contributor` | Prompts and completions are used to train and improve Meta models; rate-limited by tokens | $0.10 / $0.002 / $0.20 |

Choose the Contributor model only when the organization accepts its data-use
terms. The distinction is part of the model ID, so changing tiers is an explicit
model selection rather than a hidden provider setting.

The previous `muse-spark-1.2` and `muse-spark-1.2-contributor` IDs remain
available with the same tiers and pricing.

## Links

- [Meta Model API documentation](https://dev.meta.ai/docs/overview/)
- [Muse Spark model page](https://developer.meta.com/ai/models/muse-spark/)
- [Migrate between providers](/how-to/migrate-providers/)
