---
type: Specification
title: "fetchkit"
description: "fetchkit library powering the `web_fetch` capability."
tags:
  - everruns
  - execution
---
# fetchkit

External library ([github.com/everruns/fetchkit](https://github.com/everruns/fetchkit)) powering the `web_fetch` capability. Provides HTTP fetching, focused content extraction, bounded crawl discovery, HTML-to-markdown conversion, SSRF protection, file download, and opt-in lightweight rendering.

## Integration

- `WebFetchCapability` uses `fetchkit::ToolBuilder` to configure the tool
- `WebFetchTool` wraps `fetchkit::Tool`, delegates schema, description, llmtxt, and execution
- All metadata (description, system prompt, input schema) comes from `fetchkit::ToolBuilder`, not constants
- The wrapper deserializes FetchKit's request contract so content focus,
  conditional fetch, bounded crawl, and rendering inputs reach the library
  without a second field-by-field contract that can drift.
- **Egress path (default in the runtime)**: when `ToolContext.egress_service`
  is present, fetchkit's `HttpTransport` (fetchkit >= 0.4) is injected via
  `ToolBuilder::transport` with `EgressHttpTransport`
  (`integrations/web-fetch/src/egress_transport.rs`). fetchkit keeps the
  whole pipeline, specialized fetchers (GitHub, Wikipedia, arXiv, …), SSRF
  via `DnsPolicy` (resolve-then-check; pinned addresses forwarded to
  `EgressRequest.pinned_addrs`), per-hop redirect validation, bot-auth
  signing, body caps, while every HTTP hop crosses the egress boundary,
  which enforces the network access list and system allowlist.
- **Direct path** (no egress service in context, e.g. embedded hosts):
  fetchkit owns transport; SSRF via `DnsPolicy::block_private_ips()` (blocks
  loopback, RFC1918, link-local, cloud metadata). Crawl requests are rejected
  when a network access list or system allowlist is active because this path
  cannot re-check policy for discovered pages.
- The system allowlist and network access list are pre-checked on the initial
  URL in the tool for clear user-facing errors on both paths.
- See `integrations/web-fetch/src/lib.rs`

## Agent-oriented fetching

FetchKit owns the structured fetchers, focused extraction, page quality signals,
and crawl result model. Everruns exposes those inputs and returns the upstream
response unchanged:

- `content_focus: "agent"` selects FetchKit's best low-noise extraction strategy.
- `crawl: true` discovers a bounded, same-origin page set; `max_pages` is bounded
  by FetchKit's schema and implementation.
- Conditional request inputs and response metadata allow callers to avoid
  refetching unchanged content.
- Specialized fetchers return compact structured content for supported source
  types; unsupported URLs continue through the default fetcher.

## Rendered fetch

The `render-rakers` Cargo feature is compiled in and the backend is enabled on
the tool, but each request must explicitly pass `render: "rakers"`. This is
lightweight JavaScript/DOM execution for pages whose useful content is produced
inline; it is not a full browser.

Rendered fetch preserves the normal initial-request URL, DNS, egress, timeout,
and body-size policy. FetchKit denies renderer-initiated subresource requests,
caps rendered output before conversion, and applies a per-script execution
timeout. This prevents rendered HTML from creating a second path around the
host egress boundary.

## File download (`FileSaver`)

fetchkit owns the `FileSaver` abstraction; consumers inject implementations:
- **CLI**: `LocalFileSaver` (real filesystem, ships with fetchkit)
- **Everruns**: `SessionFileSaver` adapter → `SessionFileSystem` (per-session virtual filesystem)

Key decisions:
- **Config-gated**: file download enabled via per-capability config `{"enable_file_download": true}`, harnesses/agents opt in
- **ToolBuilder-driven**: `enable_save_to_file` on ToolBuilder controls schema, description, and system prompt content
- **Binary encoding**: UTF-8 validity check (`std::str::from_utf8`) determines text vs base64, simpler than content-type heuristics
- **Binary content accepted**: `save_to_file` bypasses binary rejection in `DefaultFetcher`

## Capability config mechanism

`WebFetchCapability` implements `tools_with_config` and `system_prompt_contribution_with_config` on the `Capability` trait. These methods read the per-capability config JSON during capability collection, enabling file download when `enable_file_download: true` is set. Generic and Chat harnesses set this config alongside `session_file_system`.

## Bot-auth (request signing)

fetchkit supports Ed25519 request signing per RFC 9421 (HTTP Message Signatures), gated behind the `bot-auth` cargo feature. When enabled, every outbound HTTP request is signed with `Signature`, `Signature-Input`, and optionally `Signature-Agent` headers.

### Configuration

Server-wide via environment variables:

| Variable | Required | Description |
|----------|----------|-------------|
| `BOT_AUTH_SIGNING_KEY_SEED` | yes (to enable) | base64url-encoded 32-byte Ed25519 seed |
| `BOT_AUTH_AGENT_FQDN` | no | FQDN for `Signature-Agent` header (key discovery) |
| `BOT_AUTH_VALIDITY_SECS` | no | signature validity window, default 300 |

When `BOT_AUTH_SIGNING_KEY_SEED` is set, all `web_fetch` requests are signed by fetchkit on both paths: signing happens before each hop is handed to the transport (re-signed per redirect hop), so it applies equally when the hop crosses the egress boundary. Egress-routed hops additionally request `EgressSigning::PlatformDefault`, a no-op until a platform egress signer exists; when one lands, signing policy should consolidate behind `EgressService` for tenant/agent runtime fetches (see `knowledge/operations/egress.md`).

Generate a seed: `python3 -c "import os, base64; print(base64.urlsafe_b64encode(os.urandom(32)).rstrip(b'=').decode())"`

### Integration

- `bot-auth` feature enabled on the fetchkit dependency in `crates/core/Cargo.toml`
- `WebFetchCapability::from_env()` reads env vars and passes `BotAuthConfig` to all `WebFetchTool` instances
- Signing failures are non-blocking: requests proceed without signature headers, warning logged

### Key discovery

Public key identity is a JWK Thumbprint (RFC 7638), available via `derive_bot_auth_public_key()`.

Target servers discover public keys via the well-known endpoint (draft-meunier-http-message-signatures-directory):

```
GET /.well-known/http-message-signatures-directory
```

Returns a JWKS (RFC 7517) with the server's Ed25519 public key. The `Signature-Agent` FQDN in outbound requests tells target servers where to look up the key.

- **Endpoint**: public, no auth, derived from `BOT_AUTH_SIGNING_KEY_SEED` at startup
- **Key derivation**: `derive_bot_auth_public_key(seed) -> BotAuthPublicKey`
- See `crates/server/src/api/http_signing_keys.rs`, `integrations/web-fetch/src/lib.rs`

## Future: archive extraction (`FilesSaver`)

Planned: `FilesSaver` trait (extends `FileSaver`) with `save_and_extract()` for zip/tar.gz/tar. Separate trait, consumer opt-in. Not yet in fetchkit.
