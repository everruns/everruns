---
title: Browserless
description: Headless browser automation through Browserless for screenshots, DOM reading, scraping, page interaction, and persistent sessions for login-protected pages.
appliesTo: [platform, cloud]
---

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="52.0" height="52.0" aria-hidden="true" style="float: right; margin-left: 16px;"><rect x="2.5" y="4" width="19" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="1.6"/><path d="M2.5 8.5h19" fill="none" stroke="currentColor" stroke-width="1.6"/><circle cx="5.6" cy="6.25" r=".75" fill="currentColor"/><circle cx="7.8" cy="6.25" r=".75" fill="currentColor"/></svg>

| | |
|---|---|
| **ID** | `browserless` |
| **Category** | Browser |
| **Features** | None |
| **Dependencies** | [`session_storage`](/capabilities/session-storage/) |

Cloud browser automation powered by [Browserless](https://www.browserless.io/). Agents can navigate pages, take screenshots, read rendered DOM content, scrape structured data, and interact with pages using click, type, keyboard, mouse, and touch events.

## Set up

1. In the [Browserless Dashboard](https://www.browserless.io/account/home), open **API Keys** in your account settings and copy your API token.
2. In Everruns, open **Settings** > **Connections**, find **Browserless**, select **Connect**, and paste the token.

Once connected, agents with the Browserless capability can use the tools below.

## Two operating modes

**Stateless (default).** Each tool call launches a fresh browser that is destroyed after the response. Nothing persists between calls, and no cleanup is needed. Use it for one-shot screenshots or scraping.

**Persistent session (CDP).** `browserless_open_browser` creates a browser over the Chrome DevTools Protocol that stays alive between tool calls, keeping login state, cookies, and navigation history. Tools use the CDP session automatically when one is active and the REST API otherwise. `browserless_scrape` always uses the REST API. A CDP browser expires after 60 seconds of inactivity by default; call `browserless_close_browser` when done for immediate cleanup.

A typical flow for a login-protected page:

1. `browserless_open_browser` with the login page URL.
2. `browserless_interact` to fill credentials and submit the form.
3. `browserless_navigate` to browse authenticated pages.
4. `browserless_screenshot` to capture the authenticated page state.
5. `browserless_close_browser` to release resources.

## Tools

### `browserless_open_browser`

Open a persistent browser session via CDP.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `url` | string | no | Initial URL to navigate to |
| `timeout_ms` | integer | no | How long the browser stays alive between calls (default: 60000) |

### `browserless_close_browser`

Close the persistent browser session and release resources. Takes no parameters.

### `browserless_navigate`

Navigate to a URL and return page metadata (title, links, headings, meta tags).

| Parameter | Type | Required | Description |
|---|---|---|---|
| `url` | string | yes | The URL to navigate to |
| `wait_for_selector` | string | no | Wait for this CSS selector to appear |
| `wait_for_timeout` | integer | no | Wait this many milliseconds after page load |

### `browserless_screenshot`

Take a PNG screenshot of a page. Returns base64-encoded image data.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `url` | string | yes | The URL to screenshot |
| `full_page` | boolean | no | Capture the full scrollable page (default: true) |
| `selector` | string | no | CSS selector to screenshot a specific element |
| `wait_for_selector` | string | no | Wait for this CSS selector before taking screenshot |
| `wait_for_timeout` | integer | no | Wait this many milliseconds before taking screenshot |

### `browserless_content`

Get the fully rendered HTML content (DOM) of a page, including JavaScript-rendered content.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `url` | string | yes | The URL to read |
| `wait_for_selector` | string | no | Wait for this CSS selector before reading content |
| `wait_for_timeout` | integer | no | Wait this many milliseconds before reading content |
| `best_attempt` | boolean | no | Continue even if async events fail or time out (default: false) |

### `browserless_scrape`

Extract structured data from a page using CSS selectors. Returns JSON with matched elements.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `url` | string | yes | The URL to scrape |
| `elements` | array | yes | Array of `{selector}` objects to extract |
| `wait_for_selector` | string | no | Wait for this CSS selector before scraping |
| `wait_for_timeout` | integer | no | Wait this many milliseconds before scraping |

### `browserless_interact`

Navigate to a URL, then perform a sequence of actions.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `url` | string | yes | The initial URL to navigate to |
| `steps` | array | yes | Ordered list of interaction steps |
| `return_screenshot` | boolean | no | Return a base64 screenshot after all steps (default: false) |
| `return_content` | boolean | no | Return DOM content after all steps (default: false). With both flags true, both are returned; with neither, DOM content is returned |

**Supported step actions:**

| Action | Key Parameters | Description |
|---|---|---|
| `click` | `selector` or `x`,`y` | Click element or coordinates |
| `type` | `selector`, `value` | Type text into input field |
| `keyboard` | `key` | Press a key (Enter, Tab, Escape, etc.) |
| `mouse_move` | `x`, `y` | Move mouse to coordinates |
| `touch` | `selector` | Tap element (mobile touch simulation) |
| `scroll` | `value` | Scroll page by pixel amount |
| `wait` | `wait_ms` | Wait for milliseconds |
| `wait_for_selector` | `selector`, `wait_ms` | Wait for element to appear |
| `navigate` | `value` | Navigate to a different URL |

## Uses

- **Accessibility testing**: navigate pages, read the DOM, check ARIA attributes and heading structure.
- **Regression testing and visual QA**: take before and after screenshots and verify content after changes.
- **Login flows**: authenticate in a persistent session and test protected pages.
- **Web scraping**: extract structured data from a website.

## Security

- API tokens are encrypted at rest (AES-256-GCM envelope encryption).
- Browser sessions are isolated on Browserless servers.
- CDP session state stores only the WebSocket endpoint (no secrets), scoped per session.
- Large DOM responses are truncated to 100KB to prevent context flooding.

## See also

- [Browserless documentation](https://docs.browserless.io/)
- [Integrations](/integrations/): every vendor integration.
- [Capabilities Overview](/capabilities/)
