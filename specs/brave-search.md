# Brave Search Capability Specification

## Abstract

The Brave Search capability integrates [Brave Search](https://brave.com/search/api/) web search as an agent tool. Agents can search the web and get relevant results including titles, URLs, and descriptions. Stateless — no per-resource state management needed.

**Status**: Experimental (Dev only)

## Architecture

### Single API

Brave Search exposes one API layer:

1. **Web Search API** (`https://api.search.brave.com/res/v1/web/search`) — query the web. Auth: `X-Subscription-Token: <API_KEY>`.

```
┌────────────────────────────────────────────┐
│              Agent Session                  │
│                                             │
│  Tool Call (brave_web_search)              │
│         ↓                                   │
│  Resolve API key:                          │
│    1. User connections (brave_search)       │
│    2. Session secret (BRAVE_SEARCH_API_KEY) │
│         ↓                                   │
│  ┌─────────────────────────────────────┐   │
│  │ BraveSearchClient                   │   │
│  │  - Web Search API                   │   │
│  └─────────────────────────────────────┘   │
│         ↓                                   │
│  Return results to agent                   │
└────────────────────────────────────────────┘
```

### API Key Resolution

The API key is resolved lazily at tool execution time:

1. **User connections** (preferred): `connection_resolver.get_connection_token(session_id, "brave_search")`. User stores their API key via `PUT /v1/user/connections/api-key/brave_search`.
2. **Session secret** (fallback): `storage_store.get_secret(session_id, "BRAVE_SEARCH_API_KEY")`.
3. **Error**: Guidance to configure in Settings > Connections or via session secret.

## Tools

### brave_web_search

Search the web using Brave Search API.

- **Parameters**:
  - `query`: string (required) — search query
  - `count`: integer (optional, 1-20, default: 10) — number of results
  - `offset`: integer (optional) — pagination offset
  - `freshness`: string (optional) — time filter: `pd` (past day), `pw` (past week), `pm` (past month), `py` (past year)
- **Returns**: `{ query, results: [{title, url, description, age?}], count }`

## Security

- **API Key**: Resolved via user connections (encrypted at rest) or session secrets (encrypted at rest)
- **No secrets in tool results**: API key never appears in tool results or message history
- **Rate limiting**: Deferred to Brave Search API (returns 429 on rate limit)

## Error Handling

| Scenario | Result Type | Message |
|----------|-------------|---------|
| Missing query param | `ToolError` | "Missing required parameter: query" |
| API key not configured | `ToolError` | "Brave Search API key not configured..." |
| HTTP 401 | `ToolError` | "Brave Search API error (401): ..." |
| HTTP 429 | `ToolError` | "Brave Search API error (429): ..." |
| No context | `ToolError` | "brave_web_search requires context." |

## User Connection

### PUT /v1/user/connections/api-key/brave_search

Store a Brave Search API key as an encrypted user connection.

**Request:**
```json
{
  "api_key": "BSA..."
}
```

**Response:** `204 No Content`

The API key is encrypted using envelope encryption (AES-256-GCM) before storage.

### DELETE /v1/user/connections/brave_search

Disconnect. Removes the stored API key.

**Response:** `204 No Content`

## Design Decisions

### API key in user connections (not just session secrets)

User connections are persistent across sessions and user-scoped. Session secrets are per-session and lost when the session ends. User connections are the preferred storage for long-lived API keys.

### Stateless (no per-resource state)

Unlike Daytona (which manages sandbox lifecycle), Brave Search is stateless. Each search is independent. No sandbox state, no session state beyond the API key.

### No dependencies

Unlike Daytona (which depends on `session_storage`), Brave Search has no capability dependencies. The API key resolution uses the connection resolver and storage store from ToolContext directly.

## Testing

### Unit & mock tests

Run without flags — no API key needed:

```bash
cargo test -p everruns-integrations-brave-search
```

Uses `wiremock` for HTTP-level assertions (auth headers, query params, error codes).

### Integration tests (real API)

Gated behind the `integration` Cargo feature so they never compile in normal `cargo test` runs:

```bash
BRAVE_SEARCH_API_KEY=<key> cargo test -p everruns-integrations-brave-search --features integration
```

Tests: `smoke_basic_search`, `smoke_freshness_filter`, `smoke_pagination` in `tests/smoke_real_api.rs`.

Tests use a `require_api_key!` macro that gracefully skips (returns early with a message) when `BRAVE_SEARCH_API_KEY` is unset or empty. This prevents CI failures when the secret is not yet configured while still compiling the test code.

### CI

Dedicated workflow `.github/workflows/brave-search-integration.yml`:

- **Path-filtered**: only triggers when `integrations/brave-search/**` changes.
- Reads `BRAVE_SEARCH_API_KEY` from GitHub Actions secrets.
- Passes `--features integration` to compile and run the real-API tests.
- If the secret is missing, tests compile and pass (skipped), ensuring CI stays green.

Adding a new API-key-gated integration crate should follow this pattern: feature-gate the tests, use a skip macro for missing keys, add a path-filtered workflow, store the key in GitHub secrets.

## Crate Structure

`integrations/brave-search/` → `everruns-integrations-brave-search`

External integration crate, auto-registered via `inventory::submit!` plugin system.

**Force-link required**: Both `crates/server/src/lib.rs` and `crates/worker/src/lib.rs` must contain `extern crate everruns_integrations_brave_search;`.

| File | Purpose |
|------|---------|
| `src/lib.rs` | Plugin registration, constants, `BraveSearchCapability` impl |
| `src/client.rs` | `BraveSearchClient` HTTP client, API response types |
| `src/tools.rs` | `BraveWebSearchTool` implementation, API key resolution |
| `tests/plugin_registration.rs` | Integration tests for inventory registration and dev/prod gating |
| `tests/smoke_real_api.rs` | Real-API smoke tests (behind `integration` feature) |

## Capability Registration

- **ID**: `brave_search`
- **Name**: `[Experimental] Brave Search`
- **Status**: Available (Dev only)
- **Icon**: `search`
- **Category**: `Network`
- **Dependencies**: none
