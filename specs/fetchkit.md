# fetchkit

External library ([github.com/everruns/fetchkit](https://github.com/everruns/fetchkit)) powering the `web_fetch` capability. Provides HTTP fetching, HTML-to-markdown conversion, SSRF protection, and file download.

## Integration

- `WebFetchCapability` uses `fetchkit::ToolBuilder` to configure the tool
- `WebFetchTool` wraps `fetchkit::Tool` — delegates schema, description, llmtxt, and execution
- All metadata (description, system prompt, input schema) comes from `fetchkit::ToolBuilder`, not constants
- SSRF: `DnsPolicy::block_private_ips()` (default) blocks loopback, RFC1918, link-local, cloud metadata
- See `crates/core/src/capabilities/web_fetch.rs`

## File download (`FileSaver`)

fetchkit owns the `FileSaver` abstraction; consumers inject implementations:
- **CLI**: `LocalFileSaver` (real filesystem, ships with fetchkit)
- **Everruns**: `SessionFileSaver` adapter → `SessionFileStore` (per-session virtual filesystem)

Key decisions:
- **Config-gated**: file download enabled via per-capability config `{"enable_file_download": true}` — harnesses/agents opt in
- **ToolBuilder-driven**: `enable_save_to_file` on ToolBuilder controls schema, description, and system prompt content
- **Binary encoding**: UTF-8 validity check (`std::str::from_utf8`) determines text vs base64 — simpler than content-type heuristics
- **Binary content accepted**: `save_to_file` bypasses binary rejection in `DefaultFetcher`

## Capability config mechanism

`WebFetchCapability` implements `tools_with_config` and `system_prompt_contribution_with_config` on the `Capability` trait. These methods read the per-capability config JSON during capability collection, enabling file download when `enable_file_download: true` is set. Generic and Chat harnesses set this config alongside `session_file_system`.

## Bot-auth (request signing)

fetchkit supports Ed25519 request signing per RFC 9421 (HTTP Message Signatures), gated behind the `bot-auth` cargo feature. When enabled, every outbound HTTP request is signed with `Signature`, `Signature-Input`, and optionally `Signature-Agent` headers.

### Integration

- `bot-auth` feature enabled on the fetchkit dependency in `crates/core/Cargo.toml`
- `BotAuthConfig` passed to `fetchkit::Tool::builder().bot_auth(config)` when configured
- Config parsed from per-capability config: `{"bot_auth": {"signing_key_seed": "...", "agent_fqdn": "...", "validity_secs": 300}}`
- Signing failures are non-blocking: requests proceed without signature headers, warning logged

### Capability config

```json
{
  "bot_auth": {
    "signing_key_seed": "<base64url-encoded 32-byte Ed25519 seed>",
    "agent_fqdn": "bot.example.com",
    "validity_secs": 300
  }
}
```

- `signing_key_seed` (required): base64url-encoded 32-byte Ed25519 seed. Store encrypted via envelope encryption.
- `agent_fqdn` (optional): FQDN for the `Signature-Agent` header, enabling bot identity discovery.
- `validity_secs` (optional): signature validity window in seconds, default 300.

### Key identity and discovery

Public key identity is a JWK Thumbprint (RFC 7638) of the Ed25519 public key, available via `BotAuthConfig::keyid()` and `derive_bot_auth_public_key()`.

Target servers discover public keys via the well-known endpoint (draft-meunier-http-message-signatures-directory):

```
GET /.well-known/http-message-signatures-directory
```

Returns a JWKS (RFC 7517) with all active Ed25519 public keys. The `Signature-Agent` FQDN in outbound requests tells target servers where to look up keys.

### Server-side components

- **Migration**: `013_http_signing_keys.sql` — `http_signing_keys` table (org_id, key_id, public_key JWK, label, expires_at)
- **Endpoint**: `GET /.well-known/http-message-signatures-directory` — public, no auth, returns JWKS
- **Key derivation**: `derive_bot_auth_public_key(seed) -> BotAuthPublicKey` — derives Ed25519 JWK + key_id from seed
- **Storage**: `upsert_http_signing_key` / `list_http_signing_keys` / `delete_http_signing_key` — CRUD for key directory
- See `crates/server/src/api/http_signing_keys.rs`, `crates/core/src/capabilities/web_fetch.rs`

## Future: archive extraction (`FilesSaver`)

Planned: `FilesSaver` trait (extends `FileSaver`) with `save_and_extract()` for zip/tar.gz/tar. Separate trait, consumer opt-in. Not yet in fetchkit.
