---
title: Web Fetch
description: Fetch a URL and convert its HTML to markdown or plain text, with SSRF protection and structured fetchers for common developer and research sources.
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

## Write-only credentials

Use `$NAME` in the exact header or form field authorized by a credential binding.
`credential_usage` explicitly selects `service` (default) or the verified `user`.
It never falls back to another identity. For example:

```json
{
  "url": "https://mail.example/api/messages",
  "method": "POST",
  "credential_usage": "user",
  "headers": { "Authorization": "Bearer $MAIL_TOKEN" },
  "json": { "to": "recipient@example.com", "subject": "Hello" }
}
```

To capture an authentication response without showing its token to the model,
declare every expected secret field:

```json
{
  "url": "https://mail.example/oauth/token",
  "method": "POST",
  "credential_usage": "user",
  "form": { "grant_type": "refresh_token", "refresh_token": "$MAIL_REFRESH" },
  "capture_secrets": [
    {
      "name": "MAIL_TOKEN",
      "from": "/access_token",
      "scope": "virtual_user",
      "binding": {
        "url": "https://mail.example/api",
        "auth": { "type": "bearer" }
      },
      "expires_in_from": "/expires_in"
    }
  ]
}
```

The user approves persistent storage, replacement, and cross-origin bindings
before any request is sent. The result contains `$MAIL_TOKEN` instead of the
value. Up to eight captures save together; malformed JSON, duplicate keys,
missing fields, interrupted/oversized responses, or storage failures withhold
the response and save no partial values. Credential requests have a 256 KiB
response limit and cannot crawl, render, extract, or save files.

Bindings support bearer headers, named headers, and named form fields
(`{"type":"form_field","name":"refresh_token"}`). References in URLs,
arbitrary bodies, JSON values, or mixed strings are rejected. Redirects are
returned without following them. Known literal, JSON-escaped, and form-encoded
token echoes are redacted before the result enters chat or events. Declare all
claim tokens, assertions, access tokens, and refresh tokens the service returns;
arbitrary encodings such as base64 are outside this redaction guarantee.

The same binding is what Bashkit HTTP uses. A command inside a Daytona, E2B, or
Modal sandbox runs without it: those machines do not receive `$NAME`, and their
outbound HTTPS is not rewritten with the secret. Modal can still inject a
GitHub connection token outside the VM; that path is separate from this binding.

Manage personal credentials in **Settings > Connections**, service
credentials on their virtual user, agent credentials on the agent's
**Credentials** tab, organization credentials in **Settings > Organization**,
and session credentials from the session's **Credentials** action. Values are
write-only. Expired or revoked bindings fail with renewal/reconnect guidance.
