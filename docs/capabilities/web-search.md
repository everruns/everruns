---
title: Web Search
description: One web_search tool answered by the search provider you choose, with free search as the fallback.
appliesTo: [framework, platform, cloud]
---

| | |
|---|---|
| **ID** | `web_search` |
| **Category** | Network |
| **Features** | None |
| **Dependencies** | None |

Web Search gives an agent one `web_search` tool. Where a search goes is a setting on the
capability, not a different tool: the agent keeps the same tool, prompts and history when you
switch providers or models. The Worker harnesses include it, so a new worker agent can search with
no key and no setup; a Conversation agent adds it on the agent.

## Providers

| Setting | What answers | Cost |
|---|---|---|
| **Organization default** (`org_default`) | The organization's default provider (see below); Automatic for a new organization | Depends on the provider |
| **Automatic** (`auto`) | The organization's Automatic order: the model's own search, then the organization's search accounts, then the deployment's own search when it offers one, then free search | Depends on the provider that answers |
| **Model provider search** (`native`) | The model's own search, on OpenAI, Anthropic, Gemini and OpenRouter models that have it (see below). Other models use free search. | Billed by the model provider |
| **Brave Search** (`brave`) | The Brave Search API with a user, session or application key, else the organization's first Brave account | Billed on the Brave key |
| **Parallel** (`parallel`) | The Parallel Search API (`fast` mode) with a user, session or application key, else the organization's first Parallel account | Billed on the Parallel key, about $1 per 1,000 searches |
| **A search account** (`account`, with `account_id`) | One organization search account, such as "Brave Search · Team", "Parallel · Research" or a [Cloudflare Web Search](/integrations/cloudflare-web-search/) account on Ceramic, Exa or Linkup | Billed on that account's key, or for Cloudflare on its AI Gateway credits ($0.25 per 1,000 searches on Ceramic) |
| **Free search** (`free`) | DuckDuckGo, then DuckDuckGo Lite, then Mojeek, then Wikipedia | Free |

Free search is the last fallback of every setting: when the chosen provider has no key, rejects
it, is rate limited or is unavailable, free search answers and the run records why. Set
`fallback` to `none` to use only the chosen provider.

## Model provider search

When model provider search comes first in an agent's plan, the model searches with its provider's
own search instead of calling `web_search`: the provider runs the searches inside the model call
and the answer cites its sources. This is decided on every turn, for the model running that turn
and the organization's current defaults, so a session that switches models keeps searching:

| Model provider | Native search |
|---|---|
| OpenAI, Azure OpenAI, ChatGPT Plan (framework hosts) | OpenAI web search, on models that have it (GPT-4o, GPT-4.1, GPT-5 and later, o3, o4-mini) |
| OpenRouter | The model vendor's own search, on models OpenRouter runs it for (OpenAI GPT-4.1 and GPT-5 and later, Claude, Gemini 3, Grok 4, Perplexity) |
| Anthropic | Anthropic web search, on Claude 3.5 Haiku, Claude 3.7 Sonnet and Claude 4 and later, up to 5 searches per model call |
| Gemini | Grounding with Google Search, on Gemini 2.0 Flash, 2.5 and 3 and later, when `web_search` is the agent's only tool |
| Others | None: the next provider in the plan answers |

The agent's domain filters and the organization's blocked domains go with each request. A provider
that cannot apply them leaves the `web_search` tool in place, and the next provider in the plan
answers: OpenAI search cannot exclude sites, so an agent or organization with blocked domains
searches with its next provider on OpenAI models. Anthropic takes allowed or blocked sites but not
both at once, and Gemini takes neither. Gemini also cannot yet search natively next to the agent's
other tools, so an agent with more tools than `web_search` searches with its next provider on
Gemini models. On OpenRouter, native search is only requested for models that have it, so
OpenRouter's own fallback search never answers in place of the plan.

Each native search is billed by the model provider: about $10 per 1,000 searches on OpenAI and
Anthropic, $14 per 1,000 search queries on Gemini 3 and $35 per 1,000 grounded answers on Gemini
2.x. The model call
records it as a `hosted_tool` cost component (`web_search_call`, with the number of searches) and
budgets count it. In chat, a native search shows as a **Searching the web for "…"** row.

The OpenAI and OpenRouter server tools capabilities keep their own `web_search` option. When one of
them already turns on its provider's search, that configuration is used.

## In the UI

**Registers > Web search** has two tabs, like Models:

- **Providers**: a card per way the organization can search. **Model provider search** lists which
  connected model providers offer it and its price; each search account shows whether its key is
  set and its place in Automatic; a deployment that pays for a search provider itself (Everruns
  search on Everruns Cloud) shows it with its price; **Free search** is always there. Admins connect an account with
  **Connect search provider**, and test, rotate or disconnect it from its card.
- **Defaults**: the default provider, the Automatic order (move entries up or down; free search
  stays last), **Allow model provider search** with its cost, what happens when a provider fails,
  results per search, and blocked domains.

In the agent editor, expand **Web search** under Capabilities to pick the provider (Organization
default, Automatic, Model provider search, a search account or Free search). **How this agent will
search** says, for the agent's default model, which provider answers first, what it costs and what
comes next. Results per search, freshness, fallback and domain filters follow.

In chat, each search is one **Searched "…"** row with the result count, the provider that
answered, the duration, and a note when an earlier provider failed. Expand it to see the top
results, then open **Details and raw response** for every attempt and each provider's verbatim
response. In Session Trace, a `web_search` step lists its attempts, with a failed one in red.

## Organization search accounts and defaults

An organization admin connects search accounts as organization connections
(`POST /v1/orgs/{org}/organization-connections/providers/brave_search`, `.../parallel` or
`.../cloudflare_web_search`), the
same way sandbox provider accounts are connected. Every agent of the organization can then search with them; the
key never reaches the agent.

`PATCH /v1/orgs/{org}` with `web_search_defaults` sets the organization's defaults:

| Field | Default | Description |
|---|---|---|
| `provider` | `auto` | What `org_default` agents use: `auto`, `native`, `brave`, `parallel`, `account` (with `account_id`) or `free` |
| `automatic_order` | `["native", "free"]` | Order of Automatic: `native`, search account ids and `hosted` (the deployment's own search, when it offers one). Free search is always last; entries left out keep their place (native first, other accounts after the listed ones, `hosted` just before free search) |
| `allow_native` | `true` | Whether agents may use the model provider's own search (billed by the model provider) |
| `fallback` | `next` | Fallback for agents that do not set their own |
| `results_per_search` | 8 | Results per search when neither the model nor the agent asks, 1 to 20 |
| `blocked_domains` | none | Sites no search returns, on top of each agent's `blocked_domains` |

Changes reach running sessions from their next search, within 30 seconds. `GET
/v1/search-providers` lists what the organization can search with, and `POST
/v1/search-providers/preview` shows the plan an agent's setting resolves to for a model.

For `brave`, a key is read from the user's Brave Search connection, then the
`BRAVE_SEARCH_API_KEY` session secret, then (Framework only) the `BRAVE_SEARCH_API_KEY`
environment variable, then the organization's first Brave Search account. `parallel` reads keys
the same way: the user's Parallel connection, then the `PARALLEL_API_KEY` session secret, then
(Framework only) the `PARALLEL_API_KEY` environment variable, then the organization's first
Parallel account.

Parallel searches its own web index and returns longer excerpts than Brave, so results carry an
`excerpt` the model can quote. It applies the agent's allowed or blocked sites and freshness
itself.

A [Cloudflare Web Search](/integrations/cloudflare-web-search/) account searches the index it was
connected with (Ceramic, Exa or Linkup) and returns at most 10 results. The API has no site or
date filters, so allowed and blocked sites are applied to its results and freshness is not.

Free search is best effort. Search engines sometimes answer automated traffic with a bot check;
free search treats that as a failure and moves to the next source instead of reporting no
results. Answers are cached for ten minutes, and requests to one search engine are spaced out.

## Settings

| Field | Default | Description |
|---|---|---|
| `provider` | `org_default` | One of the settings above |
| `account_id` | none | The search account, with `provider: account` |
| `fallback` | organization's | `next` falls back to the next provider, then free search; `none` stops |
| `limit` | organization's (8) | Results per search, 1 to 20 |
| `freshness` | none | `day`, `week`, `month` or `year` |
| `allowed_domains` | none | Only results from these sites |
| `blocked_domains` | none | Never results from these sites |
| `show_provider` | `false` | Tell the model which provider answered and why |

```json
{"ref": "web_search", "config": {"provider": "brave", "blocked_domains": ["pinterest.com"]}}
```

## One search per agent

Provider-specific search capabilities stay available for agents built around one provider:
Brave Search, DuckDuckGo, Parallel Search and Fetch, Parallel Paid Search, and the OpenAI and
OpenRouter server tools with their `web_search` tool on. Each says whose search it is. An agent that has two of these, or one of them next to an MCP server's `web_search` tool,
gets a `capabilities.duplicate_web_search` warning from [Agent Checks](/features/agent-checks/).
Two search tools make the model guess which one to use. The warning never blocks a save.

## Tools

### `web_search`

| Parameter | Type | Required | Description |
|---|---|---|---|
| `query` | string | yes | What to search for |
| `limit` | integer | no | How many results, 1 to 20 |
| `freshness` | string | no | `day`, `week`, `month` or `year` |
| `domains` | string[] | no | Only results from these sites; cannot widen the agent's own filters |
| `topic` | string | no | `dev` also searches GitHub, npm and crates.io (free search) |

Returns the query and a list of results, each with an `id`, `title`, `url`, `site`, and when known
`published`, `snippet` and `excerpt`. The model cites a claim with the result's `url` or `id`; an
`id` stays the same for the same URL throughout a session. Free search adds an `answer` when an
encyclopedia abstract matches.

To show those citations as numbered source chips in the chat, also add
[Web Citations](/capabilities/citation-web/) (`citation_web`). It covers model provider search
too, from the citations the provider returns.

Which provider answered, every attempt with its reason, and each provider's raw response are kept
on the tool call for people (the run's details and Session Trace). The model sees them only with
`show_provider`.

## Framework

Enable the `web-search` feature and add the capability:

```rust ignore
use everruns::WebSearch;

let agent = Agent::builder()
    // ...
    .capability(WebSearch::auto())
    .build()?;
```

`WebSearch::brave().no_fallback()`, `WebSearch::parallel()`, `WebSearch::free()`, `.limit(5)` and
`.blocked_domains(["pinterest.com"])` set the other options. The builder writes only config;
put a Brave key in `BRAVE_SEARCH_API_KEY` and a Parallel key in `PARALLEL_API_KEY`.
