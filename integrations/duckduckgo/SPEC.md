# DuckDuckGo Capability Specification

## Abstract

The DuckDuckGo capability integrates [DuckDuckGo Instant Answer API](https://api.duckduckgo.com/api) as an agent tool. Agents can get instant answers, abstracts (from Wikipedia and other sources), definitions, and related topics. Stateless — no per-resource state management needed.

**Status**: Experimental (Dev only)

## Architecture

### Single API

DuckDuckGo exposes one API layer:

1. **Instant Answer API** (`https://api.duckduckgo.com/`) — get instant answers. Auth: none (free, no API key required).

```
┌────────────────────────────────────────────┐
│              Agent Session                  │
│                                             │
│  Tool Call (duckduckgo_search)             │
│         ↓                                   │
│  No API key needed                         │
│         ↓                                   │
│  ┌─────────────────────────────────────┐   │
│  │ DuckDuckGoClient                    │   │
│  │  - Instant Answer API               │   │
│  └─────────────────────────────────────┘   │
│         ↓                                   │
│  Return results to agent                   │
└────────────────────────────────────────────┘
```

### No API Key Required

The DuckDuckGo Instant Answer API is free and requires no authentication. This makes it simpler than Brave Search — no connection resolver or session secrets needed. The tool does not require context (`requires_context() = false`).

## Tools

### duckduckgo_search

Get instant answers from DuckDuckGo.

- **Parameters**:
  - `query`: string (required) — search query
  - `no_html`: boolean (optional, default: true) — strip HTML from result text
- **Returns**: Object with available fields:
  - `query`: the original query
  - `type`: response type — `article`, `disambiguation`, `category`, `name`, `exclusive`, or `nothing`
  - `heading`: topic heading (if available)
  - `abstract`: `{ text, source, url }` — topic summary from Wikipedia or other sources
  - `answer`: `{ text, type }` — direct answer (calculations, IP lookups, etc.)
  - `definition`: `{ text, source, url }` — dictionary definition
  - `related_topics`: array of `{ text, url }` — related topics (flattened from groups, max 10)
  - `results`: array of `{ text, url }` — official/direct results

## Security

- **No API key**: The DuckDuckGo Instant Answer API is free and public. No secrets to manage.
- **Rate limiting**: Deferred to DuckDuckGo API. No formal rate limit documented, but excessive use may be throttled.

## Error Handling

| Scenario | Result Type | Message |
|----------|-------------|---------|
| Missing query param | `ToolError` | "Missing required parameter: query" |
| HTTP error | `ToolError` | "DuckDuckGo API error ({status}): ..." |
| Network error | `ToolError` | "Failed to connect to DuckDuckGo API: ..." |

## Design Decisions

### No API key (unlike Brave Search)

DuckDuckGo Instant Answer API is free and public. No connection resolver or session secrets needed. This simplifies the integration significantly and means `requires_context() = false`.

### Instant answers (not web search results)

DuckDuckGo does not offer an official API for full organic web search results. The Instant Answer API returns curated content: abstracts, definitions, calculations, and related topics. For comprehensive web search, agents should use Brave Search. DuckDuckGo complements Brave Search for quick facts and definitions.

### Stateless (no per-resource state)

Like Brave Search, DuckDuckGo is stateless. Each query is independent.

### no_html defaults to true

HTML in results is stripped by default since agents process plain text. The parameter is exposed in case HTML content is desired.

### Related topics limited to 10

The API can return many related topics, especially for disambiguation queries. We cap at 10 to keep tool output reasonable for LLM context windows.

## Testing

### Unit & mock tests

Run without flags — no API key needed:

```bash
cargo test -p everruns-integrations-duckduckgo
```

Uses `wiremock` for HTTP-level assertions.

### Integration tests (real API)

Gated behind the `integration` Cargo feature so they never compile in normal `cargo test` runs:

```bash
cargo test -p everruns-integrations-duckduckgo --features integration
```

Tests: `smoke_basic_search`, `smoke_calculation_answer`, `smoke_no_html_mode`, `smoke_disambiguation` in `tests/smoke_real_api.rs`.

No API key is needed even for integration tests (the DuckDuckGo API is free), but we gate them to avoid hitting the API in every CI run.

### CI

Dedicated workflow `.github/workflows/duckduckgo-integration.yml`:

- **Path-filtered**: only triggers when `integrations/duckduckgo/**` changes.
- No secrets needed (free API).
- Passes `--features integration` to compile and run the real-API tests.

## Crate Structure

`integrations/duckduckgo/` → `everruns-integrations-duckduckgo`

External integration crate, auto-registered via `inventory::submit!` plugin system.

**Force-link required**: Both `crates/server/src/lib.rs` and `crates/worker/src/lib.rs` must contain `extern crate everruns_integrations_duckduckgo;`.

| File | Purpose |
|------|---------|
| `src/lib.rs` | Plugin registration, constants, `DuckDuckGoCapability` impl |
| `src/client.rs` | `DuckDuckGoClient` HTTP client, API response types |
| `src/tools.rs` | `DuckDuckGoSearchTool` implementation, response formatting |
| `tests/plugin_registration.rs` | Integration tests for inventory registration and dev/prod gating |
| `tests/smoke_real_api.rs` | Real-API smoke tests (behind `integration` feature) |

## Capability Registration

- **ID**: `duckduckgo`
- **Name**: `[Experimental] DuckDuckGo`
- **Status**: Available (Dev only)
- **Icon**: `search`
- **Category**: `Network`
- **Dependencies**: none
