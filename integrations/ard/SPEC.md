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

### Runtime attach seam (current limitation, follow-up)

The MCP-server config overlay and external-A2A-agent config are resolved
**statically at turn start** (`load_turn_context` reads an immutable
`session.mcp_servers`; `SpawnAgentTool` captures the external-agent list at
instantiation). `SessionMutator` exposes only `update_session_title()`, and
`ToolContext` has no API to mutate session-scoped `mcpServers` or the external
A2A agent list mid-session.

Consequently this increment records the verified, SSRF-checked attachment in the
session resource registry (the only runtime-mutable seam a tool has) for
visibility and idempotency. Runtime *consumption* — so a freshly attached MCP
server's tools or A2A agent become callable in the same session — requires a new
core seam (e.g. `SessionMutator::upsert_session_mcp_servers` /
`upsert_external_agents` plus turn-start re-resolution). That plumbing is a
deliberate follow-up; it does not exist today, and inventing a fragile hack was
explicitly out of scope.

## Security review

| Threat | Control |
|---|---|
| SSRF via attacker-chosen URL | Model picks a configured `registry_id`; never a raw URL. Resolved endpoint validated with the shared `validate_safe_url` (loopback/RFC1918/link-local/metadata/CGNAT/IPv4-mapped-IPv6 blocked). |
| DNS rebinding on the resolved URL | `validate_safe_url` is the write-time check; execution-time DNS-pinned validation (`validate_url_dns_pinned`) belongs to the consuming MCP/A2A client at call time (deferred with the runtime seam). |
| Untrusted/malicious registry content | All `/search` and `/resolve` payloads treated as untrusted; envelopes must be value-or-reference; unsupported media types rejected; no registry text is executed. |
| Spoofed identity | Trust gate: when `require_trust`, a manifest with an attestation must exist and its identity must match the URN authority domain (subdomain match only). |
| Resource sprawl / DoS | `max_attachments` cap enforced before remote work; idempotent per URN. |
| Local/internal exposure in dev | `allow_local_urls` bypasses only local-address blocking and defaults false. |

Cryptographic signature verification of the attestation is structural-only in
this increment (presence + identity match) and is a documented follow-up.

## Parity status

Implemented: SPEC, unit tests, wiremock integration tests, user docs, CI unit
job + path filter, threat-model section. Deferred follow-ups: connection
provider + live API tests + dedicated live workflow, seed agent, UI test case,
cryptographic attestation verification, runtime consumption seam.
