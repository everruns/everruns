---
title: Browserless
description: Cloud browser automation for screenshots, scraping, and testing with Browserless
---

Everruns integrates with [Browserless](https://www.browserless.io/) to provide cloud-based browser automation. Agents can navigate web pages, take screenshots, read DOM content, scrape structured data, and interact with UI elements (click, type, keyboard, mouse, touch).

## What You Get

- **Screenshots**: Capture full-page or element-specific PNG screenshots
- **DOM Reading**: Get fully rendered HTML including JavaScript-generated content
- **Structured Scraping**: Extract data from pages using CSS selectors
- **Browser Interactions**: Click, type, press keys, use mouse/touch events
- **Persistent Sessions**: Keep a browser alive across tool calls for login-protected pages (CDP mode)

## Quick Start

### 1. Get Your API Token

1. Go to the [Browserless Dashboard](https://cloud.browserless.io)
2. Navigate to **API Keys** in your account settings
3. Copy your API token

### 2. Connect in Everruns

1. Go to **Settings** > **Connections**
2. Find **Browserless** in the available providers
3. Click **Connect** and paste your API token

Once connected, the Browserless capability is automatically available in agent sessions.

### 3. Use in Sessions

Agents with the Browserless capability can use these tools:

| Tool | Description |
|------|-------------|
| `browserless_open_browser` | Open a persistent browser session (CDP mode) |
| `browserless_close_browser` | Close the persistent browser session |
| `browserless_navigate` | Navigate to a URL and get page metadata |
| `browserless_screenshot` | Take a PNG screenshot of a page |
| `browserless_content` | Get the fully rendered HTML/DOM content |
| `browserless_scrape` | Extract structured data via CSS selectors |
| `browserless_interact` | Multi-step interactions (click, type, keyboard, mouse, touch) |

## Two Operating Modes

### Stateless Mode (Default)

Each tool call launches a fresh browser that is destroyed after the response. No state persists between calls. Best for one-shot operations like screenshots or scraping.

### Persistent Session Mode (CDP)

Use `browserless_open_browser` to create a persistent browser via Chrome DevTools Protocol. The browser stays alive between tool calls, preserving login state, cookies, and navigation history. Use `browserless_close_browser` when done.

**Example workflow for login-protected pages:**
1. `browserless_open_browser` with the login page URL
2. `browserless_interact` to fill credentials and submit the form
3. `browserless_navigate` to browse authenticated pages
4. `browserless_screenshot` to capture authenticated page state
5. `browserless_close_browser` to release resources

## Use Cases

- **Accessibility testing** — Navigate pages, read DOM, check ARIA attributes and heading structure
- **Regression testing** — Screenshot pages and verify content after changes
- **Login flows** — Use persistent sessions to authenticate and test protected pages
- **Web scraping** — Extract structured data from any website
- **Visual QA** — Take before/after screenshots to verify UI changes

## Resource Management

- **Stateless mode**: No cleanup needed — browsers are ephemeral
- **CDP mode**: Browsers auto-expire after 60 seconds of inactivity. Always call `browserless_close_browser` when done for immediate cleanup.

## Security

- API tokens are encrypted at rest (AES-256-GCM envelope encryption)
- Browser sessions are fully isolated on Browserless servers
- CDP session state stores only the WebSocket endpoint (no secrets), scoped per session
- Large DOM responses are truncated to 100KB to prevent context flooding

## Links

- [Browserless Website](https://www.browserless.io/)
- [Browserless Dashboard](https://cloud.browserless.io)
- [Browserless Documentation](https://docs.browserless.io/)
