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
| **Organization default** (`org_default`) | The organization's choice. Until organizations can set one, this is Automatic. | Depends on the provider |
| **Automatic** (`auto`) | Brave Search when a Brave key is available, otherwise free search | Brave bills its key; free search costs nothing |
| **Model provider search** (`native`) | The model's own search. Until model provider search ships, free search answers. | Billed by the model provider |
| **Brave Search** (`brave`) | The Brave Search API | Billed on the Brave key |
| **Free search** (`free`) | DuckDuckGo, then DuckDuckGo Lite, then Mojeek, then Wikipedia | Free |

Free search is the last fallback of every setting: when the chosen provider has no key, rejects
it, is rate limited or is unavailable, free search answers and the run records why. Set
`fallback` to `none` to use only the chosen provider.

A Brave key is read from the user's Brave Search connection, then the `BRAVE_SEARCH_API_KEY`
session secret, then (Framework only) the `BRAVE_SEARCH_API_KEY` environment variable.

Free search is best effort. Search engines sometimes answer automated traffic with a bot check;
free search treats that as a failure and moves to the next source instead of reporting no
results. Answers are cached for ten minutes, and requests to one search engine are spaced out.

## Settings

| Field | Default | Description |
|---|---|---|
| `provider` | `org_default` | One of the settings above |
| `fallback` | `next` | `next` falls back to the next provider, then free search; `none` stops |
| `limit` | 8 | Results per search, 1 to 20 |
| `freshness` | none | `day`, `week`, `month` or `year` |
| `allowed_domains` | none | Only results from these sites |
| `blocked_domains` | none | Never results from these sites |
| `show_provider` | `false` | Tell the model which provider answered and why |

```json
{"ref": "web_search", "config": {"provider": "brave", "blocked_domains": ["pinterest.com"]}}
```

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
