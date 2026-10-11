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

Status: approved (2026-10-11), being built in the delivery order below. Step 1 (the
capability, free and Brave drivers, harnesses, Framework builder) is in place.

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
default for new organizations. Because native search costs money (about $10 per
1,000 searches, billed by the model provider), every place that offers it says
so in plain words, and when the model runs through the Everruns LLM provider the
searches are booked on the organization's ledger like tokens.

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

**Search plan.** The ordered list of providers a call will try: for example
`[native, brave(account 7f…), free]`. It is resolved live from the agent's
setting and the current organization defaults, not pinned at session start: an
admin who connects Brave or reorders Automatic changes running sessions from
their next search. The worker caches the organization defaults for 30 seconds,
so a change lands within that window. Each attempt records the plan it used, so
the Trace still shows why a given search went where it did.

**Search driver.** The common abstraction under every function-executed
provider, in the same spirit as LLM drivers: `SearchDriver` in
`everruns-contracts` with `id()`, `capabilities()` (freshness, domain filters,
excerpt length, pagination) and `search(SearchRequest) -> SearchResponse`. Free
search, Brave and Parallel are drivers in `everruns-integrations`; Exa and
Tavily later are one driver each. A `SearchDriverRegistry` maps provider ids to
drivers the way the connector registry maps connectors. Native search is not a
`SearchDriver`: it is a per-LLM-driver adapter (below) because the model
provider runs it inside the model call.

## The tool

One model-facing tool for every provider:

```text
web_search(query, limit?, freshness?: day|week|month|year, domains?: [..])
```

Function-executed providers (free, Brave, Parallel) return one shape, built
for citing:

```json
{
  "query": "tokio 1.50 release notes",
  "results": [
    {"id": "w3f9a1c", "title": "Announcing Tokio 1.50", "url": "https://tokio.rs/blog/2026-10-tokio-1-50",
     "site": "tokio.rs", "published": "2026-10-02", "snippet": "…", "excerpt": "…"}
  ],
  "provider": "brave",
  "notes": ["Brave Search was rate limited; answered by free search."]
}
```

- `id` is derived from the URL (`w3f9a1c`), so the same page keeps the same
  id across every search in a session; the model can cite a result by id and
  the citation feed can resolve it later; `url`, `title`,
  `site` and `published` are what a reader needs to check a claim, and
  `excerpt` (when the driver has one, as Parallel and Brave extra snippets do)
  gives the model enough text to quote.
- The tool description tells the model to cite with the result `url` (or `id`)
  next to the claim it supports.
- **Which provider answered** is always recorded on the tool event for people
  and the Trace. Whether the model also sees it is a capability setting,
  `show_provider` (off by default): on, the result carries `provider` and
  fallback `notes`, so an agent can say "according to Brave Search" or notice
  that free search answered.
- Results are untrusted page content and are marked as such, exactly like
  `web_fetch` output.

Native search never reaches the tool executor: the driver replaces the
`web_search` function definition with the provider's hosted tool for that
request. The provider runs the search and the model answers with citations. The
driver turns the hosted call into the existing `tool.hosted_call` event with the
same `query` and `results` fields, so the chat and Trace render one row type
whichever provider answered (see the chat mockup below).

Native results are normalized to the same result entries (with `id`s), and
the provider's own citations become annotations on the assistant text (see
Citations below).

The tool name stays `web_search` across model switches, so a session that moves
from Claude to Mistral keeps its history valid: earlier native calls replay as
the provider's own blocks where the provider requires it, and as plain text
summaries otherwise.

## Choosing a provider

Agent setting (capability config), one of:

- **Organization default** (the default for a new capability instance)
- **Automatic**
- **Model provider search**: native first; on a model without it, free
  search answers (see fallback below).
- **A named search account**, for example "Brave Search · Team"
- **Free search**

Plus optional `limit`, `freshness`, `allowed_domains` and `blocked_domains`.
Organization blocked domains always apply on top of the agent's.

Organization defaults (a `web_search_defaults` column on `organization_settings`):

- default provider (same choices except "Organization default"; new orgs get Automatic)
- Automatic order: native, then accounts in the order the admin sets, then free
  search, which is always last and cannot be removed
- allow model provider search (on by default)
- on failure: fall back to the next provider in the plan, or stop. The
  default is to fall back, and **free search is the default last fallback of
  every plan**, not only Automatic: an agent set to Brave gets
  `[brave, free]`, one set to Model provider search gets `[native, free]`. An
  agent can opt out with `fallback: none` when it must only use the provider it
  names (for example to keep every search on a paid index).
- results per search (default 8, cap 20)
- blocked domains

Resolution happens live: each turn decides whether native applies to the model
running it (that is what makes Automatic follow a mid-session model switch), and
each function call resolves the rest of the plan from the current agent setting
and organization defaults.

### Why native comes first in Automatic

Native search is usually the best result for the model that asked: the provider
tunes it for that model, it returns cited answers, and it needs no setup. Its
costs are real (about $10 per 1,000 searches on Anthropic and OpenAI, more for
some OpenAI context sizes), so the order is editable and the whole kind can be
turned off. An admin who wants every search on the organization's own Brave key
drags Brave above it or turns native off.

The price is never hidden. The provider card, the Automatic order, the agent
editor's "How this agent will search" box and the Defaults switch all show the
per-search price of the model provider in use ("Anthropic search, billed by
Anthropic, about $10 per 1,000 searches"); the chat row's detail view shows the
cost of each native call.

### Cost accounting and the Everruns LLM provider

Every native call already produces a hosted-call cost component on its
`llm.generation` (the OpenAI path does this today); the Anthropic, Gemini and
OpenRouter adapters add theirs from the provider's usage fields
(`server_tool_use.web_search_requests`, grounding metadata, OpenRouter's
reported cost). Budgets count it like tokens.

When the model runs through the Everruns LLM provider (the hosted, prepaid
service in SaaS), the provider's bill for those searches lands on Everruns'
upstream account, so it must reach the organization's ledger: the usage meter
books each native search as its own charge line, service `llm`, meter
`web_search_call`, priced from the rate card at the upstream price plus the
usual margin, next to the tokens of the same generation. The Usage tab shows
"Web searches" as its own row. Bring-your-own-key providers are not booked;
their searches are billed by the provider to the customer directly, and only
appear as cost on the generation and in budgets. Brave and Parallel on the
organization's own key are likewise not booked, only counted.

### Network allowlist

Free search and search accounts run in the worker and pass through the egress
policy like `web_fetch`. Search must never be blocked by it, in any policy mode
(`open`, `curated-writes`, `curated-all`):

| Driver | Hosts | In `system_allowlist.toml` today |
|---|---|---|
| Free search | `html.duckduckgo.com`, `lite.duckduckgo.com`, `duckduckgo.com` | yes (`*.duckduckgo.com`, group `search`) |
| Free search fallback | `www.mojeek.com` | **no: add `*.mojeek.com` to group `search`** |
| Free search, developer queries | `api.github.com`, `registry.npmjs.org`, `crates.io` | yes (`*.github.com`, `*.npmjs.org`, `*.crates.io`) |
| Free search, answer enrichment | `api.duckduckgo.com`, `*.wikipedia.org` | yes |
| Brave | `api.search.brave.com` | yes (`*.brave.com`) |
| Parallel | `api.parallel.ai` | yes (`*.parallel.ai`) |
| Exa, Tavily (later) | `api.exa.ai`, `api.tavily.com` | yes |

Each driver declares its hosts in code (`SearchDriver::egress_hosts()`), and a
unit test checks every declared host against the compiled system allowlist,
so a driver that adds a host without allowlisting it fails CI rather than
production. Free search's requests count toward the per-org open-read limit
like any other read; its own rate limiter keeps it well under that.

Native searches run at the model provider and never touch the organization's
network path, so the allowlist neither blocks nor covers them. The "Allow
model provider search" switch exists for organizations whose egress rules must
cover every fetch.

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

## Feature flags, docs and icons

No flags. `web_search` and free search are opt-in through the capability, and
Brave and Parallel are opt-in through connecting an account, so the existing
`brave_search`, `duckduckgo` and `parallel_search` Adoption flags are removed
(see [Feature flags](../security/feature-flags.md)). Native search has the
organization switch above.

Every provider ships finished, not experimental: the "[Experimental]" prefix is
dropped, each provider has its own icon in the Facet family (Brave, Parallel,
model provider marks reused from the Models page, a neutral globe for Web
search and a magnifier for free search), and public docs get one page,
`docs/integrations/web-search.md`, replacing `docs/integrations/brave-search.md`:
what each provider does, its cost, how Automatic picks, how to connect a key,
the agent settings, citations, and the yolop and Framework setup.

## Built-in harnesses

Web search is part of the built-in harness tree, so every new agent can search
without adding anything. It joins the two branch layers in
[the shared preset definition](../../crates/contracts/src/capability/presets.rs),
**Conversation** and **Worker**, with config `{"provider": "org_default"}`;
Bashkit Worker and Sandbox Worker inherit it from Worker. Base stays limited to
what every agent needs to run reliably. With a fresh organization's defaults
(Automatic, fallback on), an agent searches with its model's own search when
the model has it and with free search otherwise, with no key and no setup.
Organization reconciliation updates the managed harness rows in place, as it
does for every preset change. An agent that must not reach the web removes the
capability or uses a custom harness. yolop's default coding harness gets the
same entry.

## What happens to today's capabilities

Internal code needs no backward compatibility, so the old switches are removed
and a migration rewrites agent definitions:

| Today | Becomes |
|---|---|
| `brave_search` | `web_search` with provider Brave (key from an org account, then the user's connection, then the `BRAVE_SEARCH_API_KEY` session secret, so existing setups keep working) |
| `duckduckgo` | `web_search` with provider Free search |
| `openai_server_tools` with `web_search` | `web_search` with Model provider search, domains and location carried over; the other hosted tools stay |
| `openrouter_server_tools` with `web_search` | same as above |
| `parallel_search` | kept, renamed "Parallel Search and Fetch" (see Overlap below); it also provides Parallel's fetch. Parallel as a search account uses the Search API with the org key. |

The seeded "web-researcher" agent switches to `web_search` with Organization
default.

## Overlap with other search capabilities

Some search capabilities stay because they do more than search (Parallel's
MCP pair also fetches), and MCP servers or plugins can bring their own search
tools. Two rules keep that understandable.

**A duplicate is a warning, never an error.** A new agent check,
`capabilities.duplicate_web_search` (Warning, category Cost), runs with the
other advisory checks in [Agent Checks](../evaluation/agent-checks.md) and
never blocks a save. It fires when an agent has more than one web search
source:

- `web_search` plus any capability that declares it searches the web;
- two such capabilities without `web_search`;
- an attached MCP server or plugin tool whose name ends in `web_search` or
  `search_web` next to any of the above.

Capabilities declare it through one new trait method,
`Capability::web_search_source(&config) -> bool` (default `false`), so
the check needs no hardcoded list and a config-dependent capability reports
only when its search part is on. The finding names both sources, for example
"Web search and Parallel Search and Fetch both search the web; the model
has to guess which one to use. Keep one, or set Web search to Parallel." The
agent editor shows the same text as an "Overlaps with Web search" badge on
the extra capability. Orgs can turn the rule off or change its severity like
any other built-in rule.

**Branding says whose search it is.** The provider-neutral capability is
named plainly "Web search" with the generic search icon. Every
provider-specific search capability carries its provider in the name, uses
the provider's icon, and ends its description with "For provider-neutral web
search, use Web search.":

| Capability | Name today | Name after |
|---|---|---|
| `web_search` | (new) | Web search |
| `parallel_search` | [Experimental] Parallel | Parallel Search and Fetch |
| `parallel` (payments) | unchanged name | Parallel Paid Search (pay per call) |
| MCP server and plugin tools | server name | unchanged; the badge names the server |

The tools follow the same rule: only the neutral capability owns the bare
`web_search` tool name. Parallel's MCP tools keep their
`mcp_parallel__` prefix, so a model sees which provider each one is.

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

## Narration

Search rows narrate in the existing narration helper
(`narrate_search_web`), extended so the finished phrase carries the outcome,
not just the query:

- running: "Searching the web for "tokio 1.50 release notes""
- done: "Found 8 results on tokio.rs, github.com and docs.rs"
- done after fallback: "Found 4 results with free search (Brave was rate limited)"
- empty: "No results for "…""
- failed: "Web search failed: Brave key rejected"

Native calls narrate the same way from the hosted-call event, in English and
Ukrainian like the other narration.

## Details view and Session Trace

Two levels, both on every search, native or not:

- **Details** (chat row expanded, and the tool drawer): query, provider and
  why it was chosen (the plan, which attempts failed and why), duration, cost,
  and the result list with title, site, date and link. Cited results are
  marked.
- **Full response** (a "Raw" tab in the drawer and in the Trace inspector):
  the request each provider received and the response it returned, verbatim:
  the Brave JSON, the DuckDuckGo page (truncated with a download link), the
  provider's `web_search_tool_result` / `web_search_call` / grounding block for
  native. Raw responses are stored in the tool result's `details` (on `tool.completed`), which the
  model never sees, capped at 256 KB per attempt; API keys and auth headers are
  stripped before storage.

In Session Trace, a search is a span under its turn with one child per attempt,
so a fallback shows as two attempts with the first one red.

## Citations

Web search is the `citation_web` feed of [Citations](../runtime-resources/citations.md):

- function results: when the capability is on, the post-generation annotation
  hook maps the URLs (or `w` ids) the model cited back to the result entries
  and attaches `TextAnnotation`s with origin `web_search`, title, url and
  snippet;
- native results: the Anthropic adapter reads text-block `citations` of type
  `web_search_result_location`, OpenAI reads `url_citation` annotations, Gemini
  reads grounding supports, and each becomes the same annotation.

The chat then shows the numbered source chips that retrieval citations already
use, with no web-specific UI. `citation_verification` works on these the same
way.

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
  happened; expanding it opens the details view, with the full raw response one
  tab away.
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

- Unit: the duplicate-search check over capability, MCP and plugin combinations, including a config-dependent capability with search off.
- Unit: free-search parsers against recorded DuckDuckGo, Lite and Mojeek pages,
  including a bot-check page; plan resolution as a table (agent setting × org
  defaults × model × flags); driver wire tests for each hosted tool, including
  replay after a model switch.
- Live, key-gated: one real query per account backend and per native driver.
- Example: a research agent under `examples/` on a real model, run once with
  Automatic on an Anthropic model and once on a model without native search.
- Manual UI cases for the page, the editor and the chat row.

## Delivery

1. Capability, `SearchDriver` contract and registry, free driver (ported and fixed), Brave driver, harness presets, `*.mojeek.com` allowlisted with the host coverage test,
   Framework builder; migrate `brave_search` and `duckduckgo` agents; the
   duplicate-search check and the renamed provider-specific capabilities.
2. Organization search accounts, `web_search_defaults`, live plan resolution,
   catalog and preview endpoints; flags removed.
3. UI: Web search page, agent editor, chat row with details and raw views,
   Trace spans, narration, icons, public docs page.
4. Native search: OpenAI and OpenRouter on the shared option, then Anthropic,
   then Gemini, each with cost components and citation mapping; strip
   `web_search` from the two hosted-tools capabilities.
5. Citations: `citation_web` over function and native results.
6. SaaS: book native searches on the org ledger for the Everruns LLM provider.
7. Parallel Search API driver.
8. yolop adoption after the next platform release.

## Options not taken

- **Search providers as rows in the model `providers` table**, with a new
  `Search` service kind. It would reuse the per-service default map, but Brave
  and Parallel have no models, credential sync or model review, and would show
  up on the Models page as broken model providers. Native search still reads
  the model provider registry.
- **Keep one capability per provider and add a picker on top.** Agents would
  still carry several tools with different shapes, and a model switch would
  still drop search.
- **Pin the plan at session creation**, like sandbox selection. Rejected
  (user call, 2026-10-11): changes to search defaults are rare and should take
  effect on running sessions; the per-attempt plan record keeps it debuggable.
- **Keep Adoption flags on Brave and Parallel.** Rejected (user call,
  2026-10-11): connecting an account is already the opt-in.
