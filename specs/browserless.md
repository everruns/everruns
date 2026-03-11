# Browserless Capability Specification

## Abstract

The Browserless capability integrates [Browserless](https://www.browserless.io/) cloud browser automation as an agent tool set. Agents can take screenshots, read rendered DOM, scrape structured data, and perform multi-step browser interactions (click, type, keyboard, mouse, touch) via the Browserless REST API.

**Status**: Available (All environments)

## Architecture

### Stateless REST API

Browserless REST API is inherently stateless: each request launches a fresh browser, performs one task, and destroys the session. No persistent browser instances to manage or clean up.

```
┌──────────────────────────────────────────┐
│              Agent Session                │
│                                           │
│  Tool Call (browserless_screenshot, etc.) │
│         ↓                                 │
│  Resolve API token from Connections       │
│         ↓                                 │
│  ┌───────────────────────────────────┐   │
│  │ BrowserlessClient                 │   │
│  │  - /screenshot                    │   │
│  │  - /content                       │   │
│  │  - /scrape                        │   │
│  │  - /function (interactions)       │   │
│  └───────────────────────────────────┘   │
│         ↓                                 │
│  Return result to agent                  │
└──────────────────────────────────────────┘
```

### No State Management

Unlike Daytona (which tracks sandbox lifecycle), Browserless needs no per-session state. Each tool call is self-contained. The API token is the only credential needed.

### API Token Resolution

The Browserless API token is resolved via **user connection** for the `browserless` provider (Settings > Connections). If not configured, a `ToolError` guides the user to set up in Settings.

### User Connection

Browserless registers as a `ConnectionProviderPlugin` (API-key type). Users configure their token in **Settings > Connections > Browserless**:

1. User enters API token (from [Browserless Dashboard](https://cloud.browserless.io))
2. Token validated via `GET /active` endpoint
3. Token encrypted and stored in `user_connections` table

## API Integration

### Browserless REST Endpoints

Base URL: `https://production-sfo.browserless.io`

Auth: `?token=<api_token>` query parameter on all requests.

| Method | Path | Purpose | Request Body | Response |
|--------|------|---------|-------------|----------|
| POST | `/screenshot` | Screenshot | `{ url, options, selector?, waitFor* }` | `image/png` bytes |
| POST | `/content` | Rendered HTML | `{ url, waitFor*, bestAttempt? }` | `text/html` |
| POST | `/scrape` | Structured data | `{ url, elements, waitFor* }` | `application/json` |
| POST | `/function` | Custom Puppeteer | `{ code, context? }` | Variable |

## Tools

### browserless_navigate

Open a URL and return page metadata.

- **Parameters**: `url` (required), `wait_for_selector` (optional), `wait_for_timeout` (optional)
- **Returns**: `{ title, url, status, links[], headings[], meta[] }`
- **Implementation**: Uses `/function` endpoint to extract rich metadata

### browserless_screenshot

Take a PNG screenshot of a page.

- **Parameters**: `url` (required), `full_page` (optional, default true), `selector` (optional), `wait_for_selector` (optional), `wait_for_timeout` (optional)
- **Returns**: `{ url, format, size_bytes, image_base64 }`
- **Implementation**: Uses `/screenshot` endpoint

### browserless_content

Get fully rendered HTML/DOM content.

- **Parameters**: `url` (required), `wait_for_selector` (optional), `wait_for_timeout` (optional), `best_attempt` (optional)
- **Returns**: `{ url, content, size_bytes, truncated }`
- **Implementation**: Uses `/content` endpoint. Truncates at 100KB to avoid overwhelming LLM context.

### browserless_scrape

Extract structured data using CSS selectors.

- **Parameters**: `url` (required), `elements` (required, array of `{selector}`), `wait_for_selector` (optional), `wait_for_timeout` (optional)
- **Returns**: `{ url, data }`
- **Implementation**: Uses `/scrape` endpoint

### browserless_interact

Multi-step browser interactions. Navigate to a URL, then execute a sequence of actions.

- **Parameters**:
  - `url` (required) — initial URL
  - `steps` (required) — array of interaction steps
  - `return_screenshot` (optional, default false) — return screenshot vs DOM after steps
- **Returns**: `{ title, url, screenshot? | content? }`
- **Implementation**: Generates Puppeteer code and executes via `/function` endpoint

**Supported step actions**:

| Action | Parameters | Description |
|--------|-----------|-------------|
| `click` | `selector` or `x`,`y` | Click element or coordinates |
| `type` | `selector`, `value` | Type text into input |
| `keyboard` | `key` | Press key (Enter, Tab, Escape, etc.) |
| `mouse_move` | `x`, `y` | Move mouse to coordinates |
| `touch` | `selector` | Tap element (mobile simulation) |
| `scroll` | `value` (pixels) | Scroll page vertically |
| `wait` | `wait_ms` | Wait for milliseconds |
| `wait_for_selector` | `selector`, `wait_ms` | Wait for element to appear |
| `navigate` | `value` (URL) | Navigate to different URL |

## Resource Management

**No resources to clean up.** Each Browserless REST call launches an ephemeral browser that is automatically destroyed after the response is returned. There are no persistent sessions, containers, or sandboxes to track or delete.

## Security

- **API Token**: Stored in user connections (Settings > Connections > Browserless), encrypted at rest
- **No secrets in chat**: Token resolved via connection provider, never exposed in conversation
- **Ephemeral browsers**: Each request gets a fresh browser — no cross-request data leakage
- **Content truncation**: Large DOM responses truncated to 100KB to prevent context flooding

## Error Handling

| Scenario | Result Type | Message |
|----------|-------------|---------|
| Missing required param | `ToolError` | "Missing required parameter: {name}" |
| API token not configured | `ToolError` | "Browserless API token not configured." |
| HTTP 4xx/5xx | `ToolError` | "Browserless API error ({status}): {body}" |
| No context | `ToolError` | "{tool_name} requires context." |

## Design Decisions

### REST-only, no WebSocket sessions

Browserless supports WebSocket connections for persistent Puppeteer/Playwright sessions. We chose REST-only because:
- No persistent state to manage or leak
- Each call is atomic and self-cleaning
- Simpler error handling (no session reconnection)
- The `/function` endpoint covers multi-step interactions

### /function for interactions

The Browserless REST API doesn't expose standalone click/type endpoints. The `/function` endpoint accepts custom Puppeteer code, which we generate from the structured `steps` array. This gives full flexibility while keeping the tool interface simple.

### No state management

Unlike Daytona sandboxes (which need lifecycle tracking), Browserless browsers are ephemeral. No `session_storage` dependency, no state persistence, no cleanup needed.

### Content truncation

DOM content can be very large. We truncate at 100KB to prevent flooding the LLM context window while still providing enough content for analysis.

## Crate Structure

`integrations/browserless/` → `everruns-integrations-browserless`

| File | Purpose |
|------|---------|
| `src/lib.rs` | Plugin registration, constants, `BrowserlessCapability` impl |
| `src/client.rs` | `BrowserlessClient` HTTP client (screenshot, content, scrape, function) |
| `src/connection.rs` | `BrowserlessConnectionProvider` — API-token connection plugin |
| `src/state.rs` | API token resolution, parameter helpers |
| `src/tools.rs` | 5 tool implementations + interaction code generator |
| `tests/plugin_registration.rs` | Integration tests for inventory registration |
| `tests/tool_integration.rs` | Integration tests: tool execution + wiremock |

## Capability Registration

- **ID**: `browserless`
- **Name**: `Browserless`
- **Status**: Available
- **Icon**: `browserless`
- **Category**: `Browser`
- **Risk Level**: Medium
- **Dependencies**: none

## Seeded Agent: Browser Tester

A pre-configured seed agent (`Browser Tester`) demonstrates the capability:
- **Capabilities**: `browserless`
- **Dev-only**: false
- **Use cases**: Accessibility testing, regression testing, web automation
