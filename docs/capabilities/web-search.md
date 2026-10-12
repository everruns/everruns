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
switch providers or models. The Conversation and Worker harnesses include it, so a new agent can
search with no key and no setup.

## Providers

| Setting | What answers | Cost |
|---|---|---|
| **Organization default** (`org_default`) | The organization's default provider (see below); Automatic for a new organization | Depends on the provider |
| **Automatic** (`auto`) | The organization's Automatic order: the model's own search, then the organization's search accounts, then free search | Depends on the provider that answers |
| **Model provider search** (`native`) | The model's own search. Until model provider search ships, free search answers. | Billed by the model provider |
| **Brave Search** (`brave`) | The Brave Search API with a user, session or application key, else the organization's first Brave account | Billed on the Brave key |
| **A search account** (`account`, with `account_id`) | One organization search account, such as "Brave Search · Team" | Billed on that account's key |
| **Free search** (`free`) | DuckDuckGo, then DuckDuckGo Lite, then Mojeek, then Wikipedia | Free |

Free search is the last fallback of every setting: when the chosen provider has no key, rejects
it, is rate limited or is unavailable, free search answers and the run records why. Set
`fallback` to `none` to use only the chosen provider.

## Organization search accounts and defaults

An organization admin connects search accounts as organization connections
(`POST /v1/orgs/{org}/organization-connections/providers/brave_search`), the same way sandbox
provider accounts are connected. Every agent of the organization can then search with them; the
key never reaches the agent.

`PATCH /v1/orgs/{org}` with `web_search_defaults` sets the organization's defaults:

| Field | Default | Description |
|---|---|---|
| `provider` | `auto` | What `org_default` agents use: `auto`, `native`, `brave`, `account` (with `account_id`) or `free` |
| `automatic_order` | `["native", "free"]` | Order of Automatic: `native` and search account ids. Free search is always last; entries left out keep their place (native first, other accounts after the listed ones) |
| `allow_native` | `true` | Whether agents may use the model provider's own search (billed by the model provider) |
| `fallback` | `next` | Fallback for agents that do not set their own |
| `results_per_search` | 8 | Results per search when neither the model nor the agent asks, 1 to 20 |
| `blocked_domains` | none | Sites no search returns, on top of each agent's `blocked_domains` |

Changes reach running sessions from their next search, within 30 seconds. `GET
/v1/search-providers` lists what the organization can search with, and `POST
/v1/search-providers/preview` shows the plan an agent's setting resolves to for a model.

For `brave`, a key is read from the user's Brave Search connection, then the
`BRAVE_SEARCH_API_KEY` session secret, then (Framework only) the `BRAVE_SEARCH_API_KEY`
environment variable, then the organization's first Brave Search account.

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

`WebSearch::brave().no_fallback()`, `WebSearch::free()`, `.limit(5)` and
`.blocked_domains(["pinterest.com"])` set the other options. The builder writes only config;
put a Brave key in `BRAVE_SEARCH_API_KEY`.
