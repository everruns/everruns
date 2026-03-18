---
title: Web Fetch
description: Fetch web content from any URL and convert HTML to clean markdown. Agents can read web pages, extract text, and follow links in their session workspace.
---

| | |
|---|---|
| **ID** | `web_fetch` |
| **Category** | Network |
| **Features** | None |
| **Dependencies** | None |

Fetch content from URLs and convert HTML to markdown or plain text. Powered by [fetchkit](https://github.com/everruns/fetchkit) with built-in SSRF protection.

## Tools

### `web_fetch`

Fetch a URL and return its content.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `url` | string | yes | URL to fetch |
| `method` | string | no | HTTP method (default: `GET`) |
| `as_markdown` | boolean | no | Convert HTML to markdown |
| `as_text` | boolean | no | Convert HTML to plain text |

Returns: content body, status code, metadata (size, content type, filename, last modified).

## Use Cases

- **Research** — fetch documentation, API references, or web articles
- **Data collection** — retrieve JSON/CSV data from public APIs
- **Content extraction** — convert web pages to clean markdown for processing
- **Link verification** — check if URLs are reachable and inspect response metadata

## Example

```
User: Summarize the Python 3.12 changelog

Agent:
  → web_fetch({ url: "https://docs.python.org/3/whatsnew/3.12.html", as_markdown: true })
  ← (markdown content of the changelog page)

  "Python 3.12 introduces several key changes: ..."
```

## Notes

- **Timeouts**: 1s for first byte, 30s for body. Partial content returned on body timeout.
- **Binary content**: Images, PDFs, etc. return metadata only (content type, size) — not the binary data.
- **SSRF protection**: Private IPs (loopback, RFC1918, link-local, CGNAT) are blocked by default with DNS pinning to prevent rebinding attacks.
- **Excessive newlines**: Automatically filtered from converted content.

## See Also

- [File System](/capabilities/file-system/) — save fetched content to workspace
- [Capabilities Overview](/capabilities/)
