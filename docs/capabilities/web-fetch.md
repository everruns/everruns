---
title: Web Fetch
description: Fetch a URL and convert its HTML to markdown.
appliesTo: [framework, platform, cloud]
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
| `method` | string | no | `GET` (default), `HEAD`, `POST`, `PUT`, `PATCH`, `DELETE`, or `OPTIONS` |
| `headers` | object | no | Request headers, such as `Authorization` |
| `body` | string | no | Raw request body |
| `json` | any | no | JSON request body; sets `Content-Type: application/json` |
| `form` | object | no | Form fields, sent as `application/x-www-form-urlencoded` |
| `as_markdown` | boolean | no | Convert HTML to markdown |
| `as_text` | boolean | no | Convert HTML to plain text |
| `content_focus` | string | no | Extraction mode: `full`, `main`, `readable`, or `agent` |
| `crawl` | boolean | no | Discover and fetch a bounded set of same-origin pages |
| `max_pages` | integer | no | Maximum crawl pages, including the seed (default: 5, maximum: 20) |
| `if_none_match` | string | no | ETag for a conditional request |
| `if_modified_since` | string | no | Last-Modified value for a conditional request |
| `save_to_file` | string | no | Workspace destination when file download is enabled for the capability |

Returns: content body, status code, metadata, quality signals, redirect history, and crawl summaries when requested.

### API requests

A method other than `GET` or `HEAD`, or any of `headers`, `body`, `json`, or `form`, sends one plain HTTP request instead of a page fetch. The result is the status code, response headers, and the response text as received, without markdown conversion. Redirects are returned rather than followed. This lets an agent follow a service's own instructions for agents, such as an `auth.md` agent registration followed by a token request:

```json
{"url": "https://example.com/oauth/token", "method": "POST",
 "form": {"grant_type": "client_credentials", "client_id": "..."}}
```

API requests need the host's egress service, so they pass the same network access and system egress policy checks as fetches. On deployments that restrict egress (such as Everruns Cloud), plain reads can reach any public site, while requests that send data (a method other than GET or HEAD, or a body) only reach the platform's list of approved services. Request and response bodies are limited to 256 KB, and the request times out after 30 seconds.

If your organization needs to send data to its own services on such a deployment, a platform administrator can allow it to extend the list. Org admins then add their hosts under **Settings > Organization > Outbound allowlist**, one per line (`api.example.com`, `*.example.com`, or an `https://example.com/path/` prefix, up to 50, public hostnames only). The extra hosts apply to your organization's traffic only, changes take effect within a minute, and the platform's blocked hosts stay blocked.

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
