---
type: Specification
title: "Web Search"
description: "Proposal: one web_search capability backed by search providers (free, model provider native, Brave, Parallel), chosen per agent with organization defaults and an Automatic order; shared by the server, the Framework, serve and yolop."
tags:
  - everruns
  - integrations
  - capabilities
  - web-search
  - providers
---
# Web Search

Status: proposed (2026-10-10). Nothing here is built yet.

## Abstract

An agent that needs the web gets one capability, **Web search**, and one tool,
`web_search`. Where a search actually goes is a separate choice, made the way
models and sandboxes are chosen: the organization connects **search providers**
and picks a default, and each agent either follows that default or picks its
own. Three kinds of provider exist:

| Kind | Examples | Needs | Billed |
|---|---|---|---|
| Free search | DuckDuckGo, Mojeek fallback, developer sources | nothing | nobody |
| Model provider search (native) | OpenAI, Anthropic, Gemini, OpenRouter | a model that supports it | on the model provider account |
| Search account | Brave Search, Parallel (later Exa, Tavily) | an API key, stored as an organization connection | on that key |

A fourth choice, **Automatic**, takes the first provider in an ordered list that
can serve the model running the current turn: native search when the model has
it, then the organization's search account, then free search. Automatic is the
default for new organizations.

## Why

Web search today is five unrelated switches:

- `brave_search` (tool `brave_web_search`) reads a per-user connection or a
  session secret; there is no organization-wide key.
- `duckduckgo` (tool `duckduckgo_instant_answer`) is an Instant Answer lookup,
  not a result list, so agents that "search" with it mostly get nothing back.
- `parallel_search` attaches Parallel's MCP server with its own
  `web_search`/`web_fetch` pair.
- `openai_server_tools` and `openrouter_server_tools` can turn on native search,
  but only for their own provider; an agent switched to Claude silently loses
  search, and the OpenAI path fails the turn outright on any other provider.
- Anthropic and Gemini native search do not exist at all.

Each switch has its own tool name and result shape, so prompts, evals and the
chat UI special-case all of them, and an agent author has to know which one
works with which model. yolop carries a sixth, `free_search`, a no-key search
that its docs advertise but its default harness never enables, and whose regex
scraping reports success when DuckDuckGo returns a bot check.

The ask behind this design: one configurable capability, providers like the
ones for models and sandboxes, organization defaults, native search included,
configured in the agent UI, and usable from yolop.

## Concepts

**Search provider.** Something that turns a query into results. Free search and
model provider search are built in and always listed. Search accounts are
organization connections whose connector declares the `web_search`
connector capability, the same way sandbox provider accounts declare
`sandbox_provisioning`. Brave and Parallel already have connectors; they gain
that capability and a search backend. Nothing new is stored for native search:
whether it is available is a property of the model provider and model that run
the turn.

**Web search capability.** `web_search`, first party, registered with the other
product capabilities. It contributes the `web_search` tool and, when native
search is in the plan, a driver option asking the driver to use the model's own
search instead. Risk level Medium (read-only, open world).

**Search plan.** The ordered list of providers a session will try: for example
`[native, brave(account 7f…), free]`. The server resolves an agent's setting and
the organization defaults into a plan when the session is created and pins it
into the session's capability config, the same way a sandbox selection is
pinned. Changing the organization default affects new sessions only, matching
"Changes apply to new sessions only" for every other agent setting.

## The tool

One model-facing tool for every provider:

```text
web_search(query, limit?, freshness?: day|week|month|year, domains?: [..])
```

Function-executed providers (free, Brave, Parallel) return one shape:
`{query, provider, results: [{title, url, snippet, published?, source}],
notes?}`. `provider` names who answered, and `notes` carries a short line when
the plan fell back ("Brave Search was rate limited, answered by free search").
Results are untrusted page content and are marked as such, exactly like
`web_fetch` output.

Native search never reaches the tool executor: the driver replaces the
`web_search` function definition with the provider's hosted tool for that
request. The provider runs the search and the model answers with citations. The
driver turns the hosted call into the existing `tool.hosted_call` event with the
same `query` and `results` fields, so the chat and Trace render one row type
whichever provider answered (see the chat mockup below).

The tool name stays `web_search` across model switches, so a session that moves
from Claude to Mistral keeps its history valid: earlier native calls replay as
the provider's own blocks where the provider requires it, and as plain text
summaries otherwise.

## Choosing a provider

Agent setting (capability config), one of:

- **Organization default** (the default for a new capability instance)
- **Automatic**
- **Model provider search**: native only; on a model without it the tool
  reports that search is unavailable for this model instead of guessing.
- **A named search account**, for example "Brave Search · Team"
- **Free search**

Plus optional `limit`, `freshness`, `allowed_domains` and `blocked_domains`.
Organization blocked domains always apply on top of the agent's.

Organization defaults (a `web_search_defaults` column on `organization_settings`):

- default provider (same choices except "Organization default"; new orgs get Automatic)
- Automatic order: native, then accounts in the order the admin sets, then free
  search, which is always last and cannot be removed
- allow model provider search (on by default)
- on failure: fall back to the next provider in the plan, or stop
- results per search (default 8, cap 20)
- blocked domains

Resolution at session creation turns the agent setting plus these defaults into
the pinned plan. Resolution per turn only decides one thing: whether the first
entry, native, applies to the model running this turn. That is what makes
Automatic follow a mid-session model switch.

### Why native comes first in Automatic

Native search is usually the best result for the model that asked: the provider
tunes it for that model, it returns cited answers, and it needs no setup. Its
costs are real (about $10 per 1,000 searches on Anthropic and OpenAI, billed on
the model provider account), so the order is editable and the whole kind can be
turned off. An admin who wants every search on the organization's own Brave key
drags Brave above it or turns native off.

### Network allowlist

Free search and search accounts run in the worker and pass through the egress
allowlist like `web_fetch`; their hosts are added to the curated list.
Native searches run at the model provider and never touch the organization's
network path. The "Allow model provider search" switch exists for organizations
whose egress rules must cover every fetch.

## Native search per driver

The capability writes one driver option, `everruns/web_search`, carrying the
domain filters, freshness, a per-turn search cap and an optional approximate
user location. Like the native browser toolset, it is a request, not a
requirement: a driver that cannot honor it for this model leaves the function
tool in place and the next provider in the plan answers.

| Driver | Hosted tool | Notes |
|---|---|---|
| OpenAI, Azure OpenAI, ChatGPT plan | Responses `web_search` | Reuses `openai_hosted_tools` wiring (`web_search_call` parsing, cost). |
| OpenRouter | `openrouter:web_search` with engine `native` | Reuses `OpenRouterServerToolKind::WebSearch`; OpenRouter falls back to its own engine for models without native search, which we disable so the plan decides. |
| Anthropic | server tool `web_search_*` | New: send the tool entry, parse `server_tool_use` and `web_search_tool_result` blocks, keep encrypted content for replay, map citations. Version pinned per model, like `browser_toolset_20260801`. |
| Gemini | `google_search` | New, per-model table: only models that accept `google_search` next to function tools. |
| Others (Mistral, Fireworks, Bedrock, Meta, …) | none | Function tool, next provider in the plan. |

`openai_server_tools` and `openrouter_server_tools` lose their `web_search`
entries; their other hosted tools stay where they are.

## Free search, improved

yolop's `free_search` moves into `everruns-integrations` as the free backend
and is fixed on the way:

- a real HTML parser instead of regexes, with snippets taken from the same
  result block as their title;
- bot-check and empty-page detection, so a CAPTCHA is a failure and not "0
  results";
- DuckDuckGo HTML, then DuckDuckGo Lite, then Mojeek as fallbacks;
- developer queries query GitHub, npm and crates.io in parallel and interleave
  them with web results instead of appending them after the limit;
- a short in-process cache and a per-host rate limiter, so a burst of agent
  searches does not get the worker's IP blocked;
- errors kept as diagnostics on the methodology entry instead of discarded;
- a neutral browser-like user agent.

The DuckDuckGo Instant Answer lookup becomes an enrichment inside free search
(a top "answer" entry when one exists) instead of its own capability.

## Feature flags

`web_search` and free search ship without a flag: adding the capability is
already the opt-in (see [Feature flags](../security/feature-flags.md)). Native
search has the organization switch above. Brave and Parallel accounts keep
their existing Adoption flags: when the flag is off for an organization, the
connector is hidden, the provider is not offered, and a pinned plan skips it.

## What happens to today's capabilities

Internal code needs no backward compatibility, so the old switches are removed
and a migration rewrites agent definitions:

| Today | Becomes |
|---|---|
| `brave_search` | `web_search` with provider Brave (key from an org account, then the user's connection, then the `BRAVE_SEARCH_API_KEY` session secret, so existing setups keep working) |
| `duckduckgo` | `web_search` with provider Free search |
| `openai_server_tools` with `web_search` | `web_search` with Model provider search, domains and location carried over; the other hosted tools stay |
| `openrouter_server_tools` with `web_search` | same as above |
| `parallel_search` | unchanged for now: it also provides Parallel's fetch. Parallel as a search account uses the Search API with the org key. |

The seeded "web-researcher" agent switches to `web_search` with Organization
default.

## Framework, serve and yolop

The capability, backends and plan resolution live in `everruns-integrations`
(feature `web-search`) with the contract types (`SearchPlan`, result shape,
the driver option) in `everruns-contracts`. No new crate.

Framework:

```rust,ignore
use everruns::WebSearch;

agent.capability(WebSearch::auto());                       // native, then free
agent.capability(WebSearch::brave_from_env().then_free()); // Brave, free fallback
```

serve: the agent definition's `{"ref": "web_search", "config": {"provider": "auto"}}`,
with account keys from the environment (`BRAVE_SEARCH_API_KEY`,
`PARALLEL_API_KEY`).

yolop adopts it in one change after the next release: `web_search` with
Automatic replaces `free_search` and `duckduckgo` in the default coding
harness. ChatGPT-plan and OpenAI sessions get OpenAI search, Anthropic sessions
get Claude's, everything else gets Brave when a key is configured, otherwise
free search. Settings stay in the existing `[[capabilities]]` shape:

```toml
[[capabilities]]
ref = "web_search"
provider = "brave"        # or auto | native | free | parallel
blocked_domains = ["pinterest.com"]
```

## UI

Mockups: `/mnt/project-files/web-search/` in the project thread.

- **Registries → Web search**, a new page beside Models, with **Providers** and
  **Defaults** tabs mirroring Models. Providers shows a card per provider:
  native (which connected model providers support it), each search account
  (status, last check, searches this month, role in Automatic), free search,
  and a "Connect search provider" card that opens the existing connector form.
  Defaults edits `web_search_defaults`, including the drag-ordered Automatic
  list.
- **Agent editor**: Web search gets a bespoke config editor like memory and
  guardrails: a provider picker listing Organization default, Automatic, native,
  each account and free search, then a plain-words "How this agent will search"
  box computed for the agent's default model, then limit, freshness and domain
  fields.
- **Chat and Trace**: one "Searched '…'" row for every provider, with a result
  count, the provider that answered, duration, and a fallback note when one
  happened.
- The Add capability dialog lists Web search once instead of Brave, DuckDuckGo
  and the hosted-tools capabilities.

## API

- `GET /v1/search-providers`: the catalog for the org: built-ins, which model
  providers offer native search, connectors with `web_search`, and connected
  accounts with status.
- Accounts reuse `/v1/orgs/{org}/organization-connections`, which accepts
  connectors declaring `web_search` as well as `sandbox_provisioning`.
- `PATCH /v1/orgs/{org}` takes `web_search_defaults`.
- `POST /v1/search-providers/preview`: resolves an agent's setting and model to
  a plan, for the agent editor's "How this agent will search" box, and runs an
  optional test query against one provider for the cards' test button.

## Testing

- Unit: free-search parsers against recorded DuckDuckGo, Lite and Mojeek pages,
  including a bot-check page; plan resolution as a table (agent setting × org
  defaults × model × flags); driver wire tests for each hosted tool, including
  replay after a model switch.
- Live, key-gated: one real query per account backend and per native driver.
- Example: a research agent under `examples/` on a real model, run once with
  Automatic on an Anthropic model and once on a model without native search.
- Manual UI cases for the page, the editor and the chat row.

## Delivery

1. Capability, contract types, free backend (ported and fixed), Brave backend,
   Framework builder; migrate `brave_search` and `duckduckgo` agents.
2. Organization search accounts, `web_search_defaults`, plan pinning at session
   creation, catalog and preview endpoints.
3. UI: Web search page, agent editor, chat row.
4. Native search: OpenAI and OpenRouter on the shared option, then Anthropic,
   then Gemini; strip `web_search` from the two hosted-tools capabilities.
5. Parallel Search API backend.
6. yolop adoption after the next platform release.

## Options not taken

- **Search providers as rows in the model `providers` table**, with a new
  `Search` service kind. It would reuse the per-service default map, but Brave
  and Parallel have no models, credential sync or model review, and would show
  up on the Models page as broken model providers. Native search still reads
  the model provider registry.
- **Keep one capability per provider and add a picker on top.** Agents would
  still carry several tools with different shapes, and a model switch would
  still drop search.
- **Resolve the organization default on every call** instead of pinning a plan
  at session creation. An admin change would alter running sessions mid-task,
  which no other agent setting does.
