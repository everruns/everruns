---
title: Cloudflare Web Search
description: Search Ceramic, Exa or Linkup through the Cloudflare Web Search API, billed to AI Gateway credits or your own provider key.
appliesTo: [platform]
---

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="52.0" height="52.0" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" style="float: right; margin-left: 16px;"><path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z"/></svg>

The [Cloudflare Web Search API](https://developers.cloudflare.com/web-search/) searches the web
through one of three indexes and runs every search through your AI Gateway, so searches show up
in its logs and analytics. A Cloudflare account connected as an organization search account
answers the provider-neutral [Web Search](/capabilities/web-search/) capability: agents keep
their one `web_search` tool.

## Indexes

| Index | Price per 1,000 searches | Zero data retention | Results |
|---|---|---|---|
| `ceramic` (default) | $0.25 | Yes | Long page descriptions from Ceramic's own index |
| `exa` | $7.00 | No | Highlights: the passages most relevant to the query |
| `linkup` | $5.00 | Yes | Sourced results with text snippets |

Prices are Cloudflare's list prices. Searches bill to the account's AI Gateway credits, or to a
provider key stored on the gateway when you give its alias. Each search account uses one index;
connect a second account to offer another.

## Setup

1. In the [Cloudflare dashboard](https://dash.cloudflare.com/profile/api-tokens), create an API
   token with **Account > Workers AI > Read** and **Account > AI Gateway > Read**.
2. Copy the account ID from the dashboard.
3. In **Registers > Web search**, connect **Cloudflare Web Search** and fill in:

| Field | Required | Description |
|---|---|---|
| Account ID | Yes | 32 hexadecimal characters |
| API Token | Yes | The token from step 1, stored encrypted |
| Search index | No | `ceramic` (default), `exa` or `linkup` |
| AI Gateway | No | The gateway searches run through; `default` when empty |
| Provider key alias | No | Alias of an Exa or Linkup key stored under AI Gateway > Provider Keys. When set, a missing key fails the search instead of falling back to credits |

Connecting runs one test search, so a wrong token, a missing permission, an unknown gateway or a
missing key alias is reported before the account is saved. Only the token is a secret; the
account ID, gateway, index and alias are stored as the connection's settings.

The API connection is
`POST /v1/orgs/{org}/organization-connections/providers/cloudflare_web_search` with `api_key`
(the token), `account_id`, and optionally `index`, `gateway_id` and `byok_alias`.

## How it searches

Pick the account in an agent's Web search settings, or put it in the organization's Automatic
order. A search returns at most 10 results, the API's maximum. The API has no site or date
filters, so allowed and blocked sites are applied to the results it returns, and an agent's
freshness setting is not applied (the search details say so). Like every search account, a
failure (rate limit, no credit, rejected token) moves Automatic to the next provider.
