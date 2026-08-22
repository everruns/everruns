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

Fetch content from URLs and convert HTML to markdown or plain text. Powered by [FetchKit](https://github.com/everruns/fetchkit) with built-in SSRF protection, low-noise extraction, bounded crawl discovery, and structured fetchers for common developer and research sources.

## Tools

### `web_fetch`

Fetch a URL and return its content.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `url` | string | yes | URL to fetch |
| `method` | string | no | HTTP method (default: `GET`) |
| `as_markdown` | boolean | no | Convert HTML to markdown |
| `as_text` | boolean | no | Convert HTML to plain text |
| `content_focus` | string | no | Extraction mode: `full`, `main`, `readable`, or `agent` |
| `crawl` | boolean | no | Discover and fetch a bounded set of same-origin pages |
| `max_pages` | integer | no | Maximum crawl pages, including the seed (default: 5, maximum: 20) |
| `if_none_match` | string | no | ETag for a conditional request |
| `if_modified_since` | string | no | Last-Modified value for a conditional request |
| `save_to_file` | string | no | Workspace destination when file download is enabled for the capability |

Returns: content body, status code, metadata, quality signals, redirect history, and crawl summaries when requested.

## Notes

- **Timeouts**: 1s for first byte, 30s for body. Partial content returned on body timeout.
- **Binary content**: Images, PDFs, etc. return metadata only (content type, size), not the binary data.
- **Focused extraction**: Use `content_focus: "agent"` for FetchKit's lowest-noise extraction strategy.
- **No JavaScript rendering**: Web Fetch returns the server's response as delivered. Pages that build their content client-side need a browser capability such as [Browserless](/capabilities/browserless/).
- **Crawl scope**: Crawl discovery stays on the seed URL's origin and enforces FetchKit's page limit.
- **SSRF protection**: Private IPs (loopback, RFC1918, link-local, CGNAT) are blocked by default with DNS pinning to prevent rebinding attacks.
- **Excessive newlines**: Automatically filtered from converted content.

## See Also

- [File System](/capabilities/file-system/), save fetched content to workspace
- [Capabilities Overview](/capabilities/)
