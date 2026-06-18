# Agentic Resource Discovery (ARD) client — SPEC

Status: Experimental (Dev grade only). Capability id: `resource_discovery`.

This crate lets agents discover external capabilities (MCP servers, A2A agents)
through operator-configured [Agentic Resource Discovery](https://agenticresourcediscovery.org/spec/)
registries and attach them into the session config layer. ARD is only a
*source* for the existing config/attach machinery; it does not modify the agent
loop or add database migrations.

## Architecture

```
discover_resources ──POST /search──▶ configured ARD registry ──▶ ranked hits (untrusted)
attach_resource    ──POST /resolve─▶ configured ARD registry ──▶ envelope (untrusted)
                       │
                       ├─ validate value-or-reference (reject url+data)
                       ├─ media-type → attachment kind (mcp_server | a2a_agent)
                       ├─ allow_attach_types gate
                       ├─ trust gate (require_trust): manifest + attestation + URN↔identity
                       ├─ SSRF: validate_safe_url on resolved endpoint (allow_local_urls bypass)
                       ├─ max_attachments cap
                       └─ record session resource (idempotent per URN)
list_attached_resources ──▶ session resource registry (kind in {mcp_server, external_a2a_agent})
```

Registries are referenced by operator-configured `id` only. The model never
supplies a raw registry or resource URL — this is the primary SSRF/exfiltration
control.

## Tool surface

| Tool | Args | Returns |
|---|---|---|
| `discover_resources` | `{ text, filter?, registry_id? }` | `{ registry_id, count, results: [{ urn, displayName, type, score, source, description }] }` |
| `attach_resource` | `{ urn, registry_id? }` | `{ urn, status: attached\|already_attached, kind, display_name, registry_id }` |
| `list_attached_resources` | `{}` | `{ count, resources: [{ urn, kind, display_name, status, metadata }] }` |

`registry_id` is required when more than one registry is configured; with a
single registry it defaults to that one.

## Config (`resource_discovery`)

See `config_schema()` in `src/lib.rs` for the JSON schema. Fields:

- `registries: [{ id, url, federation? }]` — operator-configured registries. `federation` is an allowlist of registry hostnames the registry may delegate to.
- `require_trust: bool` (default `true`) — trust-manifest gate before any attach.
- `allow_attach_types: ["mcp_server" | "a2a_agent"]` — empty means all supported kinds.
- `max_attachments: usize` (default `8`) — per-session attachment cap.
- `allow_local_urls: bool` (default `false`) — test/dev escape hatch that bypasses ONLY local-address blocking (never scheme/parse rejection).

## Attachment state

Attachments are recorded in the session resource registry
([`specs/session-resources.md`](../../specs/session-resources.md)):

- MCP server entries → `kind: "mcp_server"`.
- A2A agent cards → `kind: "external_a2a_agent"`.
- `resource_id` is the URN, giving idempotency (a repeat attach is a no-op
  reported as `already_attached`).
- Metadata carries `ard_urn`, `ard_registry_id`, `ard_media_type`,
  `attachment_kind`, `endpoint_url`, and `trusted`.

### Runtime attach seam

**MCP servers (implemented, EVE-593).** A successful `attach_resource` for an
MCP-server resource is now *consuming*: the attached server becomes callable by
the agent on the **next turn**. The mechanism reuses the session's existing
`mcp_servers` JSONB overlay, which the server re-resolves on every
`GetTurnContext`:

1. After the full security gauntlet (SSRF `validate_safe_url`, trust gate, kind/
   `allows_kind`, `max_attachments`) and after recording the session-resource
   entry (kept for idempotency/visibility), the tool builds a one-entry
   `ScopedMcpServers` (HTTP transport, the SSRF-validated resolved endpoint URL,
   tool discovery enabled). The logical name is derived deterministically from
   the URN as `ard_<sanitized-urn>_<fnv1a-hex>` — collision-safe and guaranteed
   to survive `sanitize_mcp_server_name` without producing the reserved `__`
   delimiter or an empty prefix (otherwise `validate_scoped_mcp_servers` would
   reject the whole merged set and silently drop all MCP tools).
2. The tool calls `SessionMutator::upsert_session_mcp_servers(session_id, overlay)`
   (from `ToolContext::session_mutator`, populated via
   `ActAtom::with_session_mutator`). This MERGES (last-wins by name) into the
   session's overlay server-side via a dedicated internal worker RPC
   (`UpsertSessionMcpServers`), which re-runs `validate_scoped_mcp_servers`
   (defense-in-depth: rejects stdio transport, unsafe URLs, dup sanitized names)
   before persisting into `session.mcp_servers`. Tenant/org scoping is preserved
   on the RPC (`Caller::internal(org_id)`) and the storage `WHERE org_id`.
3. The next `GetTurnContext` re-resolves the overlay via
   `merge_effective_scoped_mcp_servers_with_capabilities`, re-validates it, and
   builds the scoped MCP tool definitions — DNS-pinned at call time by the
   standard scoped-MCP client. No worker turn-start changes and no DB migration
   were needed (the column already exists).

The success payload carries `callable_next_turn: true` for MCP attaches.

**External A2A agents (follow-up).** A2A attach remains visibility-only: it is
recorded in the session resource registry but does NOT yet become callable in
the same session. Runtime consumption for A2A needs a new session column +
migration, a new resolution hook, and a separate mutator method
(`upsert_external_agents`) — a deliberate, documented follow-up. The
`attach_resource` payload reports `callable_next_turn: false` for A2A.

## Security review

| Threat | Control |
|---|---|
| SSRF via attacker-chosen URL | Model picks a configured `registry_id`; never a raw URL. Resolved endpoint validated with the shared `validate_safe_url` (loopback/RFC1918/link-local/metadata/CGNAT/IPv4-mapped-IPv6 blocked). |
| DNS rebinding on the resolved URL | `validate_safe_url` is the write-time check; execution-time DNS-pinned validation is inherited from the standard scoped-MCP client at call time once the server is resolved into the next turn's effective set. (For A2A, call-time pinning is deferred with the A2A runtime seam.) |
| Untrusted/malicious registry content | All `/search` and `/resolve` payloads treated as untrusted; envelopes must be value-or-reference; unsupported media types rejected; no registry text is executed. |
| Spoofed identity | Trust gate: when `require_trust`, a manifest with an attestation must exist and its identity must match the URN authority domain (subdomain match only). |
| Resource sprawl / DoS | `max_attachments` cap enforced before remote work; idempotent per URN. |
| Local/internal exposure in dev | `allow_local_urls` bypasses only local-address blocking and defaults false. |

Cryptographic signature verification of the attestation is structural-only in
this increment (presence + identity match) and is a documented follow-up.

## Parity status

Implemented: SPEC, unit tests, wiremock integration tests, user docs, CI unit
job + path filter, threat-model section, MCP-server runtime attach seam
(next-turn callable via `SessionMutator::upsert_session_mcp_servers`). Deferred
follow-ups: connection provider + live API tests + dedicated live workflow, seed
agent, UI test case, cryptographic attestation verification, external-A2A runtime
consumption seam (new column/migration + resolution hook + `upsert_external_agents`).
