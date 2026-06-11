# fetchkit

External library ([github.com/everruns/fetchkit](https://github.com/everruns/fetchkit)) powering the `web_fetch` capability. Provides HTTP fetching, HTML-to-markdown conversion, SSRF protection, and file download.

## Integration

- `WebFetchCapability` uses `fetchkit::ToolBuilder` to configure the tool
- `WebFetchTool` wraps `fetchkit::Tool` — delegates schema, description, llmtxt, and execution
- All metadata (description, system prompt, input schema) comes from `fetchkit::ToolBuilder`, not constants
- **Egress path (default in the runtime)**: when `ToolContext.egress_service`
  is present, transport goes through `EgressService`
  (`crates/core/src/capabilities/web_fetch_egress.rs`). fetchkit acts as the
  request/response adapter (schema + HTML→markdown/text conversion); SSRF uses
  `validate_url_dns_pinned` with per-hop redirect re-validation; the network
  access list and system allowlist are enforced at the egress boundary.
  Specialized fetchers (GitHub, Wikipedia, arXiv, …) do not apply on this path
  — they own private HTTP clients inside fetchkit. Restoring them requires
  upstream transport injection in fetchkit.
- **Legacy direct path** (no egress service in context, e.g. embedded hosts):
  fetchkit owns transport; SSRF via `DnsPolicy::block_private_ips()` (blocks
  loopback, RFC1918, link-local, cloud metadata); system allowlist enforced as
  a pre-flight check in the tool.
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

When `BOT_AUTH_SIGNING_KEY_SEED` is set, `web_fetch` requests on the **legacy direct path** are signed. The egress path requests `EgressSigning::PlatformDefault` instead; fetchkit's signer is not applied there because signing policy belongs behind `EgressService` so web_fetch, LLM drivers, and integrations share the same outbound signing path (a platform egress signer does not exist yet — see `specs/egress.md`).

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
