# Browserless Capability Specification

## Abstract

The Browserless capability integrates [Browserless](https://www.browserless.io/) cloud browser automation as an agent tool set. Agents can take screenshots, read rendered DOM, scrape structured data, and perform multi-step browser interactions (click, type, keyboard, mouse, touch).

Two operating modes:
- **One-shot** (default): Each tool call opens a fresh guarded browser over CDP, destroyed when the call disconnects. No state, no cleanup.
- **Persistent** (CDP sessions): `browserless_open_browser` opens a persistent browser via Chrome DevTools Protocol WebSocket. Subsequent tools reuse it, preserving login state and cookies. `browserless_close_browser` releases the browser.

Both modes use the network guard below. The REST endpoints remain only as a fallback for Browserless cloud tokens that cannot open CDP sessions.

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
│  │ CDP session      │     │ One-shot guarded browser     │   │
│  │ active?          │──no─│  connect /chromium, work,    │   │
│  │  ↓ yes           │     │  disconnect (destroyed)      │   │
│  │ Reconnect via WS │     │  ↓ CDP refused (401/403) on  │   │
│  │ Do work via CDP  │     │    Browserless cloud only    │   │
│  │ Call reconnect   │     │ REST /screenshot /content    │   │
│  │ Disconnect       │     │      /scrape /function       │   │
│  │ Store endpoint   │     └──────────────────────────────┘   │
│  └─────────────────┘                                         │
│         ↓                                                     │
│  Return result to agent                                      │
└──────────────────────────────────────────────────────────────┘
```

### CDP Session Lifecycle

The CDP session uses Browserless's `Browserless.reconnect` command to keep the browser alive between tool calls without maintaining a persistent WebSocket connection:

1. **Open**: Connect via WebSocket → create a guarded browser context and a page in it (see Network Guard) → arm `Fetch` → call `Browserless.reconnect(timeout)` → store endpoint and context id → disconnect
2. **Use**: Reconnect via stored endpoint → reattach to the page in the stored guarded context (a new guarded context if it is gone; never a page outside one) → arm `Fetch` → do work → call `Browserless.reconnect` → disconnect
3. **Close**: Reconnect → disconnect without calling reconnect → browser destroyed → clean up state

Session state (`ws_endpoint`, `browser_context_id`, timestamps) stored as plain key-value in `session_storage` (not encrypted secrets). API token is always resolved from user connection at call time, never stored in session state.

### API Token Resolution

The Browserless API token is resolved via **user connection** for the `browserless` provider (Settings > My agent experience). See `src/connection.rs` for the `ConnectionProviderPlugin`.

### User Connection

Browserless registers as a `ConnectionProviderPlugin` (API-key type). Users configure their token in **Settings > My agent experience**:

1. User enters API token (from [Browserless Dashboard](https://www.browserless.io/account/home))
2. Token validated via `GET /active` endpoint
3. Token encrypted and stored in `user_connections` table

## API Integration

### REST Endpoints

Base URL: `https://production-sfo.browserless.io` (configurable via `BROWSERLESS_API_BASE` env var)

Auth: `?token=<api_token>` query parameter on all requests.

| Method | Path | Purpose | Request Body | Response |
|--------|------|---------|-------------|----------|
| POST | `/screenshot` | Screenshot | `{ url, options, selector?, waitFor* }` | `image/png` bytes |
| POST | `/content` | Rendered HTML | `{ url, waitFor*, bestAttempt? }` | `text/html` |
| POST | `/scrape` | Structured data | `{ url, elements, waitFor* }` | `application/json` |
| POST | `/function` | Custom Puppeteer | `{ code, context? }` | Variable |

### CDP (WebSocket) Protocol

Base URL: `wss://production-sfo.browserless.io` (configurable via `BROWSERLESS_WS_BASE` env var)

New browser sessions connect to `/chromium` path (Browserless v2 requirement).
Reconnect endpoints use the path returned by `Browserless.reconnect`.

Auth: `?token=<api_token>` query parameter on WebSocket URL.

CDP commands used:
- `Target.createBrowserContext`, Create the guarded context (dead proxy, see Network Guard)
- `Target.getBrowserContexts`, Check a stored guarded context still exists
- `Target.getTargets`, Find the page in the stored guarded context
- `Target.createTarget`, Create an `about:blank` page inside the guarded context
- `Fetch.enable`, `Fetch.fulfillRequest`, `Fetch.failRequest`, `Fetch.continueRequest`, Pause and answer every page request
- `Storage.setCookies`, `Storage.getCookies`, Carry stored cookies into and out of a one-shot browser
- `Target.attachToTarget`, Attach to the active page target with `flatten: true`
- `Page.enable`, Enable page events
- `Page.navigate`, Navigate to URL
- `Page.captureScreenshot`, Take screenshot (returns base64 PNG)
- `Runtime.evaluate`, Execute JavaScript (DOM access, page info, wait logic)
- `Input.dispatchMouseEvent`, Click, mouse move
- `Input.dispatchKeyEvent`, Keyboard input
- `Input.dispatchTouchEvent`, Touch/tap simulation
- `Browserless.reconnect`, Keep browser alive after disconnect (returns new WS endpoint)

`Page.*`, `Runtime.*`, `Input.*`, `Emulation.*`, and `Fetch.*` must be sent with the attached target `sessionId` as a top-level CDP field. Browser-wide commands such as `Target.*`, `Storage.*`, and `Browserless.*` stay on the root session.

### Network Guard (EVE-1189)

A Browserless browser would otherwise resolve DNS and follow redirects itself, so checking the URL a tool receives leaves redirect hops, page requests, and DNS rebinding unchecked. The browser therefore gets no network of its own (`src/browser_egress.rs`):

1. Pages live in a browser context whose proxy is a dead loopback port (`http://127.0.0.1:9`, loopback bypass removed). A request nothing answers fails with `ERR_PROXY_CONNECTION_FAILED`. This covers requests made while no client is attached between persistent calls, WebSockets (which `Fetch` cannot pause), popups, and workers.
2. `Fetch` pauses every request at the Request stage. A background reader answers it while commands wait: static SSRF check, session network access list, and the host system allowlist when `EVERRUNS_SYSTEM_ALLOWLIST_ENABLED` is set, then Everruns performs the request with redirects disabled and `SsrfGuardResolver` refusing private DNS answers at connect time, then `Fetch.fulfillRequest`. A 3xx goes back to the browser, whose next hop is paused and checked again.
3. Request bodies come from `postDataEntries`; `accept-encoding` is pinned to `identity` and gzip/deflate bodies are decoded (Chrome does not decode a fulfilled body); bodies over 10 MB fail. Multiple `Set-Cookie` headers are kept.
4. A failed navigation (`errorText` on `Page.navigate`) is a tool error; no content or screenshot of Chrome's error page is returned.

Browserless cloud itself drops the connection when a page touches a loopback or metadata URL; that also surfaces as a tool error.

The REST endpoints run browsers with direct network access. They are used only when CDP is refused (401/403) and `BROWSERLESS_API_BASE` is a `*.browserless.io` host, whose browsers sit outside Everruns and operator networks. A self-hosted Browserless never falls back. In the fallback, a non-empty session access list travels as Browserless `rejectRequestPattern` entries (`validation::transport_reject_patterns`).

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
- **Behavior**: Reconnects and disconnects without calling `Browserless.reconnect`, browser is destroyed.

### browserless_navigate

Open a URL and return page metadata.

- **Parameters**: `url` (required), `wait_for_selector` (optional), `wait_for_timeout` (optional)
- **Returns**: `{ title, url, status, links[], headings[], meta[] }`
- **Session-aware**: Uses the persistent browser if one exists, a one-shot guarded browser otherwise.

### browserless_screenshot

Take a PNG screenshot of a page.

- **Parameters**: `url` (required), `full_page` (optional, default true), `selector` (optional), `wait_for_selector` (optional), `wait_for_timeout` (optional)
- **Returns**: `{ url, format, size_bytes, image_base64 }`
- **Session-aware**: CDP `Page.captureScreenshot` (clipped to the element for `selector`) on the persistent or a one-shot browser.

### browserless_content

Get fully rendered HTML/DOM content.

- **Parameters**: `url` (required), `wait_for_selector` (optional), `wait_for_timeout` (optional), `best_attempt` (optional)
- **Returns**: `{ url, content, size_bytes, truncated }`
- **Session-aware**: CDP `Runtime.evaluate` on the persistent or a one-shot browser. Truncates at 100KB.

### browserless_scrape

Extract structured data using CSS selectors, in the Browserless `/scrape` response shape (`Runtime.evaluate`).

- **Parameters**: `url` (required), `elements` (required, array of `{selector}`), `wait_for_selector` (optional), `wait_for_timeout` (optional)
- **Returns**: `{ url, data }`

### browserless_interact

Multi-step browser interactions.

- **Parameters**: `url` (required), `steps` (required), `return_screenshot` (optional, default false), `return_content` (optional, default false)
- **Returns**: `{ title, url, screenshot?, content? }`, when both flags are true, both fields are included. If neither is set, returns content by default.
- **Session-aware**: Runs steps as CDP commands (click, type, keyboard, mouse, touch) on the persistent or a one-shot browser. A one-shot browser saves its cookies for the next call, as the REST `/function` fallback does.

**Supported step actions**: See `src/interaction_code.rs` for REST and `execute_with_context()` for CDP.

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

### Secret References

The `browserless_interact` tool supports `${{secrets.<name>}}` placeholders in step `value` fields. This allows agents to fill login forms and other sensitive inputs without ever seeing the plaintext credentials.

**Flow:**
1. User stores credentials via `secret_store set login_password YExample0`
2. Agent references them: `{ "action": "type", "selector": "#password", "value": "${{secrets.login_password}}" }`
3. Tool resolves `login_password` from `session_secrets` at execution time, substitutes into the step, executes
4. Plaintext never appears in tool arguments, agent messages, or tool results

**Security constraints:**
- Secret refs only allowed in step `value` fields, blocked in `url` parameter and `navigate` action values to prevent exfiltration via URL
- All referenced secrets must exist; tool fails fast with a clear error if any are missing
- Supports mixed values: `"Bearer ${{secrets.api_token}}"` resolves correctly

## Resource Management

### One-shot Mode
No resources to clean up. Each call opens a browser without `Browserless.reconnect`, so Browserless destroys it when the call disconnects.

### CDP Mode
Browser stays alive on Browserless servers between tool calls via `Browserless.reconnect` with a configurable timeout (default 60s). Resources are released when:
1. Agent calls `browserless_close_browser`
2. Reconnect timeout expires (browser auto-destroyed by Browserless)
3. Session ends (stored state becomes stale, browser auto-destroyed by timeout)

No long-lived WebSocket connections from our side, we connect/disconnect for each tool call.

## Security

- **API Token**: Stored in user connections (Settings > My agent experience), encrypted at rest
- **CDP session state**: Stored as plain key-value in `session_storage` (only WS endpoint, no secrets), per-session scoped
- **No secrets in chat**: Token resolved via connection provider, never exposed in conversation
- **No secrets in logs**: CDP debug logging redacts API tokens from WebSocket URLs
- **URL validation**: Only `http://` and `https://` URLs allowed (blocks `file://`, `javascript:`, etc.). The session network access list is also checked on every navigation entry, including nested interact steps and the persistent browser's initial URL, for a clear early error.
- **Network guard**: Everruns performs every page request; private, loopback, link-local, and metadata destinations are refused as URLs and as DNS answers on every hop (see Network Guard)
- **Timeout caps**: All wait/timeout values capped at 120s to prevent unbounded resource consumption
- **Ephemeral by default**: One-shot browsers have no cross-request data leakage beyond the session's stored cookies
- **Content truncation**: Large DOM responses truncated to 100KB (UTF-8 safe boundary) to prevent context flooding
- **Secret references**: `${{secrets.*}}` resolved server-side from `session_secrets`; blocked in `url` param and `navigate` step values to prevent exfiltration via URL

## Testing

### Unit Tests (in crate)
- Client: wiremock-based HTTP tests for all REST endpoints (success + error paths)
- Tools: metadata validation, schema validation, context-required checks
- Interaction code generation: all action types, multi-step, screenshot vs content
- CDP: key-to-code mapping, guarded context and `Fetch` arming, reattach rules, paused redirect hop answered while `Page.navigate` waits
- Browser egress: static, DNS-rebinding, and access-list refusals; redirects returned not followed; bodies, cookies, and decoding
- Real Chromium (skipped without one): DNS rebinding, redirect hops, page scripts and WebSockets, and detach-time requests never reach a loopback "internal" service
- State: serialization roundtrip, reconnect URL construction
- Session tools: metadata, context-required checks

### Integration Tests (`tests/`)
- `plugin_registration.rs`: published plugin consts, dev/prod registry, capability metadata
- `tool_integration.rs`: full tool execution flow via wiremock, parameter validation, auth, error handling, resource cleanup
- `browser_network_guard.rs`: every one-shot tool fails a paused redirect hop to the metadata service without returning content; a self-hosted CDP refusal never falls back to REST

### Live API Tests
Tests against the real Browserless API require `BROWSERLESS_TOKEN` in Doppler. Gated behind `browserless-live-tests` feature flag:

```bash
doppler run -- cargo test -p everruns-integrations --features browserless-live-tests
```

CI keeps Browserless live coverage off `pull_request`. The account runs on a small credit allowance, so `.github/workflows/browserless-integration.yml` runs the live suite only on pushes to `main` that change `crates/integrations/src/browserless/**` or `crates/integrations/tests/browserless_*.rs` (docs excluded), and on manual `workflow_dispatch`. It is not part of `ci.yml` or the weekly `integration-live-sweep.yml`; `scripts/test-ci-sandbox-live-filter.sh` enforces this.

## Crate Structure

`integrations/browserless/` → `everruns-integrations`

| File | Purpose |
|------|---------|
| `src/lib.rs` | Plugin registration, constants, `BrowserlessCapability` impl |
| `src/cdp.rs` | `CdpSession`, minimal CDP client over WebSocket with a background reader |
| `src/browser_egress.rs` | `BrowserEgress`, the Everruns-side network guard answering paused requests |
| `src/client.rs` | `BrowserlessClient`, REST HTTP client |
| `src/connection.rs` | `BrowserlessConnectionProvider`, API-token connection plugin |
| `src/state.rs` | API token resolution, browser session state, parameter helpers |
| `src/session_tools.rs` | `browserless_open_browser` / `browserless_close_browser` tools |
| `src/tools.rs` | 5 session-aware tool implementations + interaction code generator |
| `tests/plugin_registration.rs` | Integration tests for the published plugin consts |
| `tests/tool_integration.rs` | Integration tests: tool execution + wiremock |

## Capability Registration

- **ID**: `browserless`
- **Name**: `Browserless`
- **Status**: Available
- **Icon**: `browserless`
- **Category**: `Browser`
- **Risk Level**: Medium
- **Dependencies**: `session_storage` (for CDP session state and secret references)

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

REST for simple one-shot operations, CDP for persistent sessions. CDP sessions preserve login state, cookies, and navigation history across tool calls, essential for testing login-protected pages.

### Minimal CDP client

Custom implementation in `cdp.rs` using `tokio-tungstenite`. No external CDP crate dependency. Implements only the CDP commands we need (Target, Page, Runtime, Input, Browserless.reconnect). Keeps the dependency footprint small.

### Reconnect pattern (not persistent WebSocket)

We don't keep long-lived WebSocket connections. Each tool call: reconnect → work → reconnect → disconnect. The browser stays alive on Browserless servers. This avoids:
- Managing WebSocket lifecycle across async tool calls
- Dealing with connection drops during LLM thinking time
- Complexity of multiplexing WebSocket messages

### Content truncation

DOM content can be very large. We truncate at 100KB to prevent flooding the LLM context window while still providing enough content for analysis.
