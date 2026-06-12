# fetchkit

External library ([github.com/everruns/fetchkit](https://github.com/everruns/fetchkit)) powering the `web_fetch` capability. Provides HTTP fetching, HTML-to-markdown conversion, SSRF protection, and file download.

## Integration

- `WebFetchCapability` uses `fetchkit::ToolBuilder` to configure the tool
- `WebFetchTool` wraps `fetchkit::Tool` — delegates schema, description, llmtxt, and execution
- All metadata (description, system prompt, input schema) comes from `fetchkit::ToolBuilder`, not constants
- **Egress path (default in the runtime)**: when `ToolContext.egress_service`
  is present, fetchkit's `HttpTransport` (fetchkit >= 0.4) is injected via
  `ToolBuilder::transport` with `EgressHttpTransport`
  (`crates/core/src/capabilities/web_fetch_egress.rs`). fetchkit keeps the
  whole pipeline — specialized fetchers (GitHub, Wikipedia, arXiv, …), SSRF
  via `DnsPolicy` (resolve-then-check; pinned addresses forwarded to
  `EgressRequest.pinned_addrs`), per-hop redirect validation, bot-auth
  signing, body caps — while every HTTP hop crosses the egress boundary,
  which enforces the network access list and system allowlist.
- **Direct path** (no egress service in context, e.g. embedded hosts):
  fetchkit owns transport; SSRF via `DnsPolicy::block_private_ips()` (blocks
  loopback, RFC1918, link-local, cloud metadata).
- The system allowlist and network access list are pre-checked on the initial
  URL in the tool for clear user-facing errors on both paths.
- See `crates/core/src/capabilities/web_fetch.rs`

## File download (`FileSaver`)

fetchkit owns the `FileSaver` abstraction; consumers inject implementations:
- **CLI**: `LocalFileSaver` (real filesystem, ships with fetchkit)
- **Everruns**: `SessionFileSaver` adapter → `SessionFileSystem` (per-session virtual filesystem)

Key decisions:
- **Config-gated**: file download enabled via per-capability config `{"enable_file_download": true}` — harnesses/agents opt in
- **ToolBuilder-driven**: `enable_save_to_file` on ToolBuilder controls schema, description, and system prompt content
- **Binary encoding**: UTF-8 validity check (`std::str::from_utf8`) determines text vs base64 — simpler than content-type heuristics
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

When `BOT_AUTH_SIGNING_KEY_SEED` is set, all `web_fetch` requests are signed by fetchkit on both paths: signing happens before each hop is handed to the transport (re-signed per redirect hop), so it applies equally when the hop crosses the egress boundary. Egress-routed hops additionally request `EgressSigning::PlatformDefault`, a no-op until a platform egress signer exists; when one lands, signing policy should consolidate behind `EgressService` so web_fetch, LLM drivers, and integrations share one outbound signing path (see `specs/egress.md`).

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
- See `crates/server/src/api/http_signing_keys.rs`, `crates/core/src/capabilities/web_fetch.rs`

## Future: archive extraction (`FilesSaver`)

Planned: `FilesSaver` trait (extends `FileSaver`) with `save_and_extract()` for zip/tar.gz/tar. Separate trait, consumer opt-in. Not yet in fetchkit.
