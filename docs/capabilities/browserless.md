---
title: Browserless
description: Cloud browser automation for screenshots, DOM reading, scraping, and interactions
---

| | |
|---|---|
| **ID** | `browserless` |
| **Category** | Browser |
| **Features** | None |
| **Dependencies** | None (session_storage used opportunistically for CDP sessions) |

Cloud browser automation powered by Browserless. Take screenshots, read DOM content, scrape structured data, and interact with web pages using click, type, keyboard, mouse, and touch events.

## Tools

### `browserless_open_browser`

Open a persistent browser session via CDP. The browser stays alive between tool calls.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `url` | string | no | Initial URL to navigate to |
| `timeout_ms` | integer | no | How long the browser stays alive between calls (default: 60000) |

### `browserless_close_browser`

Close the persistent browser session and release resources.

No parameters.

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
| `best_attempt` | boolean | no | Continue even if async events fail or timeout (default: false) |

### `browserless_scrape`

Extract structured data from a page using CSS selectors. Returns JSON with matched elements.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `url` | string | yes | The URL to scrape |
| `elements` | array | yes | Array of `{selector}` objects to extract |
| `wait_for_selector` | string | no | Wait for this CSS selector before scraping |
| `wait_for_timeout` | integer | no | Wait this many milliseconds before scraping |

### `browserless_interact`

Multi-step browser interactions. Navigate to a URL, then perform a sequence of actions.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `url` | string | yes | The initial URL to navigate to |
| `steps` | array | yes | Ordered list of interaction steps |
| `return_screenshot` | boolean | no | Return screenshot after steps (default: false = DOM content) |

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

## Authentication

Browserless API token is resolved automatically from **Settings > Connections > Browserless**.

## Use Cases

- **Accessibility testing** — read DOM, check ARIA attributes and heading structure
- **Regression testing** — screenshot pages and compare visual state
- **Login-protected testing** — use CDP sessions to authenticate and browse
- **Web scraping** — extract structured data from any website
- **Visual QA** — screenshot before/after interactions

## Notes

- Stateless mode: each tool call uses a fresh browser (no cleanup needed)
- CDP mode: browser persists across calls (close when done)
- Large DOM responses are truncated to 100KB
- Session-aware: tools automatically use CDP session when active, REST otherwise
- `browserless_scrape` always uses REST API (no CDP equivalent)

## See Also

- [Browserless integration guide](/integrations/browserless/) — setup and configuration
- [Capabilities Overview](/capabilities/)
