# Browserless Capability Specification

## Abstract

The Browserless capability integrates [Browserless](https://www.browserless.io/) cloud browser automation as an agent tool set. Agents can take screenshots, read rendered DOM, scrape structured data, and perform multi-step browser interactions (click, type, keyboard, mouse, touch).

Two operating modes:
- **REST** (default): Each tool call uses a fresh ephemeral browser. No state, no cleanup.
- **CDP** (persistent sessions): `browserless_open_browser` opens a persistent browser via Chrome DevTools Protocol WebSocket. Subsequent tools reuse it, preserving login state and cookies. `browserless_close_browser` releases the browser.

**Status**: Available (All environments)

## Architecture

### Dual-Mode Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                      Agent Session                            │
│                                                               │
│  Tool Call (browserless_screenshot, etc.)                     │
│         ↓                                                     │
│  Resolve API token from User Connections                     │
│         ↓                                                     │
│  ┌─────────────────┐     ┌──────────────────────────────┐   │
│  │ CDP session      │     │ REST Client                  │   │
│  │ active?          │──no─│  - /screenshot               │   │
│  │  ↓ yes           │     │  - /content                  │   │
│  │ Reconnect via WS │     │  - /scrape                   │   │
│  │ Do work via CDP  │     │  - /function (interactions)  │   │
│  │ Call reconnect   │     └──────────────────────────────┘   │
│  │ Disconnect       │                                         │
│  │ Store endpoint   │                                         │
│  └─────────────────┘                                         │
│         ↓                                                     │
│  Return result to agent                                      │
└──────────────────────────────────────────────────────────────┘
```

### CDP Session Lifecycle

The CDP session uses Browserless's `Browserless.reconnect` command to keep the browser alive between tool calls without maintaining a persistent WebSocket connection:

1. **Open**: Connect via WebSocket → call `Browserless.reconnect(timeout)` → store endpoint → disconnect
2. **Use**: Reconnect via stored endpoint → do work → call `Browserless.reconnect` → disconnect
3. **Close**: Reconnect → disconnect without calling reconnect → browser destroyed → clean up state

Session state (`ws_endpoint`, timestamps) stored as plain key-value in `session_storage` (not encrypted secrets). API token is always resolved from user connection at call time — never stored in session state.

### API Token Resolution

The Browserless API token is resolved via **user connection** for the `browserless` provider (Settings > Connections). See `src/connection.rs` for the `ConnectionProviderPlugin`.

### User Connection

Browserless registers as a `ConnectionProviderPlugin` (API-key type). Users configure their token in **Settings > Connections > Browserless**:

1. User enters API token (from [Browserless Dashboard](https://cloud.browserless.io))
2. Token validated via `GET /active` endpoint
3. Token encrypted and stored in `user_connections` table

## API Integration

### REST Endpoints

Base URL: `https://production-sfo.browserless.io`

Auth: `?token=<api_token>` query parameter on all requests.

| Method | Path | Purpose | Request Body | Response |
|--------|------|---------|-------------|----------|
| POST | `/screenshot` | Screenshot | `{ url, options, selector?, waitFor* }` | `image/png` bytes |
| POST | `/content` | Rendered HTML | `{ url, waitFor*, bestAttempt? }` | `text/html` |
| POST | `/scrape` | Structured data | `{ url, elements, waitFor* }` | `application/json` |
| POST | `/function` | Custom Puppeteer | `{ code, context? }` | Variable |

### CDP (WebSocket) Protocol

Base URL: `wss://production-sfo.browserless.io`

Auth: `?token=<api_token>` query parameter on WebSocket URL.

CDP commands used:
- `Page.enable` — Enable page events
- `Page.navigate` — Navigate to URL
- `Page.captureScreenshot` — Take screenshot (returns base64 PNG)
- `Runtime.evaluate` — Execute JavaScript (DOM access, page info, wait logic)
- `Input.dispatchMouseEvent` — Click, mouse move
- `Input.dispatchKeyEvent` — Keyboard input
- `Input.dispatchTouchEvent` — Touch/tap simulation
- `Browserless.reconnect` — Keep browser alive after disconnect (returns new WS endpoint)

## Tools

### browserless_open_browser

Open a persistent browser session via CDP WebSocket.

- **Parameters**: `url` (optional, initial URL), `timeout_ms` (optional, default 60000)
- **Returns**: `{ status, message, title, url, timeout_ms }`
- **Behavior**: If a session already exists and is alive, returns `already_open`. Otherwise opens a new browser.

### browserless_close_browser

Close the persistent browser session.

- **Parameters**: none
- **Returns**: `{ status, message }`
- **Behavior**: Reconnects and disconnects without calling `Browserless.reconnect` — browser is destroyed.

### browserless_navigate

Open a URL and return page metadata.

- **Parameters**: `url` (required), `wait_for_selector` (optional), `wait_for_timeout` (optional)
- **Returns**: `{ title, url, status, links[], headings[], meta[] }`
- **Session-aware**: Uses CDP if active session exists, REST `/function` otherwise.

### browserless_screenshot

Take a PNG screenshot of a page.

- **Parameters**: `url` (required), `full_page` (optional, default true), `selector` (optional), `wait_for_selector` (optional), `wait_for_timeout` (optional)
- **Returns**: `{ url, format, size_bytes, image_base64 }`
- **Session-aware**: Uses CDP `Page.captureScreenshot` if session exists, REST `/screenshot` otherwise.

### browserless_content

Get fully rendered HTML/DOM content.

- **Parameters**: `url` (required), `wait_for_selector` (optional), `wait_for_timeout` (optional), `best_attempt` (optional)
- **Returns**: `{ url, content, size_bytes, truncated }`
- **Session-aware**: Uses CDP `Runtime.evaluate` if session exists, REST `/content` otherwise. Truncates at 100KB.

### browserless_scrape

Extract structured data using CSS selectors. REST-only (no CDP equivalent).

- **Parameters**: `url` (required), `elements` (required, array of `{selector}`), `wait_for_selector` (optional), `wait_for_timeout` (optional)
- **Returns**: `{ url, data }`

### browserless_interact

Multi-step browser interactions.

- **Parameters**: `url` (required), `steps` (required), `return_screenshot` (optional, default false)
- **Returns**: `{ title, url, screenshot? | content? }`
- **Session-aware**: Uses CDP session (click, type, keyboard, mouse, touch via CDP commands) if active, generates Puppeteer code for REST `/function` otherwise.

**Supported step actions**: See `src/tools.rs:build_interaction_code()` for REST and `execute_with_context()` for CDP.

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

### REST Mode
No resources to clean up. Each call launches an ephemeral browser that is automatically destroyed.

### CDP Mode
Browser stays alive on Browserless servers between tool calls via `Browserless.reconnect` with a configurable timeout (default 60s). Resources are released when:
1. Agent calls `browserless_close_browser`
2. Reconnect timeout expires (browser auto-destroyed by Browserless)
3. Session ends (stored state becomes stale, browser auto-destroyed by timeout)

No long-lived WebSocket connections from our side — we connect/disconnect for each tool call.

## Security

- **API Token**: Stored in user connections (Settings > Connections > Browserless), encrypted at rest
- **CDP session state**: Stored as plain key-value in `session_storage` (only WS endpoint, no secrets), per-session scoped
- **No secrets in chat**: Token resolved via connection provider, never exposed in conversation
- **Ephemeral by default**: REST mode has no cross-request data leakage
- **Content truncation**: Large DOM responses truncated to 100KB to prevent context flooding

## Testing

### Unit Tests (in crate)
- Client: wiremock-based HTTP tests for all REST endpoints (success + error paths)
- Tools: metadata validation, schema validation, context-required checks
- Interaction code generation: all action types, multi-step, screenshot vs content
- CDP: key-to-code mapping
- State: serialization roundtrip, reconnect URL construction
- Session tools: metadata, context-required checks

### Integration Tests (`tests/`)
- `plugin_registration.rs`: inventory submission, dev/prod registry, capability metadata
- `tool_integration.rs`: full tool execution flow via wiremock, parameter validation, auth, error handling, resource cleanup

### Live API Tests
Tests against the real Browserless API require `BROWSERLESS_KEY` in Doppler. Gated behind `browserless-live-tests` feature flag:

```bash
doppler run -- cargo test -p everruns-integrations-browserless --features browserless-live-tests
```

## Crate Structure

`integrations/browserless/` → `everruns-integrations-browserless`

| File | Purpose |
|------|---------|
| `src/lib.rs` | Plugin registration, constants, `BrowserlessCapability` impl |
| `src/cdp.rs` | `CdpSession` — minimal CDP client over WebSocket |
| `src/client.rs` | `BrowserlessClient` — REST HTTP client |
| `src/connection.rs` | `BrowserlessConnectionProvider` — API-token connection plugin |
| `src/state.rs` | API token resolution, browser session state, parameter helpers |
| `src/session_tools.rs` | `browserless_open_browser` / `browserless_close_browser` tools |
| `src/tools.rs` | 5 session-aware tool implementations + interaction code generator |
| `tests/plugin_registration.rs` | Integration tests for inventory registration |
| `tests/tool_integration.rs` | Integration tests: tool execution + wiremock |

## Capability Registration

- **ID**: `browserless`
- **Name**: `Browserless`
- **Status**: Available
- **Icon**: `browserless`
- **Category**: `Browser`
- **Risk Level**: Medium
- **Dependencies**: none (session_storage used opportunistically for CDP state)

## Seeded Agent: Browser Tester

A pre-configured seed agent (`Browser Tester`) demonstrates the capability:
- **ID**: `0x10c`
- **Capabilities**: `browserless`
- **Dev-only**: false
- **Tags**: browser, testing, automation, a11y, regression, demo, seed
- **Use cases**: Accessibility testing, regression testing, web automation, login flows
- **System prompt**: Guides the agent through navigate → screenshot → content → scrape → interact workflows

## Design Decisions

### Dual-mode: REST + CDP

REST for simple one-shot operations, CDP for persistent sessions. CDP sessions preserve login state, cookies, and navigation history across tool calls — essential for testing login-protected pages.

### Minimal CDP client

Custom implementation in `cdp.rs` using `tokio-tungstenite`. No external CDP crate dependency. Implements only the CDP commands we need (Page, Runtime, Input, Browserless.reconnect). Keeps the dependency footprint small.

### Reconnect pattern (not persistent WebSocket)

We don't keep long-lived WebSocket connections. Each tool call: reconnect → work → reconnect → disconnect. The browser stays alive on Browserless servers. This avoids:
- Managing WebSocket lifecycle across async tool calls
- Dealing with connection drops during LLM thinking time
- Complexity of multiplexing WebSocket messages

### Content truncation

DOM content can be very large. We truncate at 100KB to prevent flooding the LLM context window while still providing enough content for analysis.
