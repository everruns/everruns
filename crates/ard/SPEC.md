# Agentic Resource Discovery (ARD), Client Capability Specification

## Abstract

The `resource_discovery` capability lets a running agent **discover and
dynamically attach external capabilities**: MCP servers and A2A agents, via
the [Agentic Resource Discovery (ARD)](https://agenticresourcediscovery.org/spec/)
protocol. This is the **client/consumer** side only. Publishing an Everruns
catalog + registry endpoint (ARD-as-a-server) is tracked separately.

**Status**: Experimental (Dev only).

ARD is a federated, search-first discovery layer: publishers host a manifest at
`/.well-known/ai-catalog.json` listing typed entries (IANA media type + URN
identifier), and registries expose `POST /search` that semantically ranks those
entries. This is the layer *above* `tool_search`:

```
ARD search  → pick capability  → attach as scoped mcpServers / external agent  → tool_search defers its schemas
(cross-org discovery)            (Everruns session config layer)                  (in-context selection)
```

`tool_search` defers schemas for tools already attached to a session; ARD
answers *"which MCP server / A2A agent should even be attached?"*

## Architecture

### Why this boundary

Attachment reuses the existing config-overlay + scoped-server + A2A machinery,
so **nothing in the agent loop changes**: ARD just becomes a *source* for
`mcpServers` / external agents. URN resolution, trust verification, and
federation handling are the only genuinely new logic.

```
┌──────────────────────── tool side (worker) ─────────────────────────┐
│ discover_resources ── POST /search ──▶ ARD registry                  │
│        │ caches ranked entries in session KV  ard_disco:{slug}       │
│        ▼                                                             │
│ attach_resource ── resolve entry ─ trust gate ─ SSRF check ─┐        │
│        writes ArdAttachment to session KV  ard_attach:{slug}─┘        │
│        registers session_resources(kind="ard_attachment")            │
└──────────────────────────────────────────────────────────────────────┘
                                  │  (session KV)
┌──────────────── turn-context assembly (server + runtime) ───────────┐
│ everruns_core::ard_attachment::apply_session_attachments()          │
│   folds attachments into the loaded Session BEFORE tools build:     │
│   • MCP  → session.mcp_servers  (scoped mcpServers, prefixed         │
│            mcp_<name>__*, subject to tool_search)                    │
│   • A2A  → session.capabilities  a2a_agent_delegation.agents[]       │
│            (usable via the existing spawn_agent flow)               │
└──────────────────────────────────────────────────────────────────────┘
```

The shared schema and merge live in `everruns_core::ard_attachment` so the
server can read/merge attachments without depending on this leaf crate. Both the
hosted server (`GetTurnContext`) and the in-process runtime (`load_resolved_turn`)
call `apply_session_attachments` after loading the session and before resolving
scoped MCP servers / capabilities. Because `GetTurnContext` reloads the session
fresh each turn, an attachment becomes live on the **next** turn.

### State management

| Data | Where | Key |
|---|---|---|
| Discovery cache (catalog entries) | session KV | `ard_disco:{urn_slug}` |
| Attachments (materialized targets) | session KV | `ard_attach:{urn_slug}` |
| Attachment visibility/audit | session resource registry | `kind = ard_attachment`, `resource_id = ard_{slug}` |

Both KV prefixes are reserved from the user-facing `kv_store` tool via
`is_internal_session_kv_key`, so a session/tool actor cannot forge attachments.
Attachments are torn down with the session (KV + registry are session-scoped).

## Tool Surface

| Tool | Behavior |
|---|---|
| `discover_resources({ text, filter?, registry_id? })` | Proxies `POST /search` to a configured registry. Returns ranked `{ urn, displayName, type, score, source, description, attachable }`. Caches each entry for later resolution. `registry_id` is optional when exactly one registry is configured. |
| `attach_resource({ urn })` | Resolves the cached entry, enforces the value-or-reference envelope, parses the URN, verifies `trustManifest` (domain ↔ URN + `require_trust`), enforces `max_attachments`, SSRF-validates the resolved URL, then materializes it (MCP → scoped `mcpServers`; A2A → external agent). Idempotent per URN. |
| `list_attached_resources()` | Lists attachments persisted for the session (for visibility/audit). |

## Capability Config

```json
{
  "registries": [
    { "id": "enterprise", "url": "https://registry.acme.com/api/v1", "federation": "referrals" },
    { "id": "public",     "url": "https://agenticresourcediscovery.org/api/v1", "federation": "none" }
  ],
  "require_trust": ["soc2"],
  "allow_attach_types": ["application/mcp-server+json", "application/a2a-agent-card+json"],
  "max_attachments": 5,
  "allow_local_urls": false
}
```

- **registries**: allowlist. The model selects a `registry_id`; raw registry
  URLs are never accepted from the model (mirrors the A2A safety rule).
- **require_trust**: attestation types required on an entry's `trustManifest`
  before it can be attached.
- **allow_attach_types**: defaults to MCP + A2A.
- **max_attachments**: per-session attach cap (default 5).
- **allow_local_urls**: permit loopback/private artifact + endpoint URLs. Tests
  / dev only; `false` in production.

## Protocol Mapping

- **Manifest / entry**: catalog entries carry `identifier` (URN),
  `displayName`, `type` (IANA media type), optional `description`/`score`/
  `source`, and a value-or-reference envelope: exactly one of `url`
  (artifact reference) or `data` (embedded artifact). Both-present or
  both-absent is rejected.
- **URN**: `urn:ai:<publisher>:<namespace...>:<name>`. The `<publisher>` FQDN
  is the trust anchor.
- **trustManifest**: `{ identity, identityType, attestations[] }`. The identity
  domain (e.g. `spiffe://acme.com/...` → `acme.com`, or `did:web:acme.com`) must
  match the URN publisher (exact or subdomain).
- **Media types**: `application/mcp-server+json` → scoped MCP server;
  `application/a2a-agent-card+json` → external A2A agent.
- **Federation**: `none | referrals | auto`, passed through per registry. We
  honor whatever federation the upstream registry returns; we do not run our own
  merge.

## Security Review

Relevant threat categories: `TM-API`, `TM-TOOL`, `TM-AGENT`, `TM-DOS` (see
`knowledge/security/threat-model.md`).

- **Registry allowlist**: model chooses a configured `registry_id`; no
  model-supplied registry URLs.
- **Trust gate**: enforced before any attach; reject entries whose
  `trustManifest` domain does not match the URN, or that lack a required
  attestation, or that omit the manifest entirely when `require_trust` is set.
- **SSRF**: `validate_url_dns_pinned` / `validate_safe_url` on every resolved
  artifact + endpoint URL (blocks loopback/private/link-local/metadata,
  DNS-pinned against rebinding). The scoped-server path re-validates on every
  subsequent MCP call.
- **`max_attachments`**: bounds blast radius / prompt-injection-driven attach
  storms.
- **Untrusted external data**: all registry-returned text (descriptions,
  queries, URNs) is treated as untrusted.
- **Forgery resistance**: `ard_attach:` / `ard_disco:` KV prefixes are reserved
  from `kv_store`.

## Auth

Connection-backed. Registry bearer token via the `ard` connection provider, with
a fallback `ARD_REGISTRY_TOKEN` session secret/env var for operator/test use.
Anonymous-read registries need no token. The registry **base URL** is supplied
via capability config (not the connection), so the connection carries only the
token.

## Testing

- **Unit** (`src/`), plugin/connector registration, config + param validation,
  URN parse + domain extraction, `trustManifest` verification (pass/mismatch/
  missing-attestation/missing-manifest), envelope value-or-reference enforcement,
  media-type → attachment-kind mapping, server-name sanitization, registry
  selection.
- **Integration** (`tests/tool_integration.rs`), `discover_resources` +
  `attach_resource` `execute_with_context` flows against a **wiremock** ARD
  registry: MCP entry → scoped `mcpServers` record; A2A entry → external agent;
  local-URL blocking unless `allow_local_urls`; trust-gate rejection;
  idempotency; discovery-required.
- **Live** (`tests/live_api_test.rs`, feature `ard-live-tests`), runs against
  the public reference registry. **Fail-closed**: when the feature is on but
  `ARD_LIVE_REGISTRY_URL` is missing/empty the test `panic!`s.

## CI

- `unit-test` job runs `cargo test -p everruns-ard`.
- `ard: crates/ard/**` path filter in the `changes` job.
- Dedicated `.github/workflows/ard.yml` (change-scoped unit + live).
- Included in `.github/workflows/integration-live-sweep.yml`.

## Example / Seed Agent

`crates/server/src/seed.rs` defines **"Capability Scout"** (`capability-scout`,
dev_only), wired with `resource_discovery` (pointed at the public reference
registry) + `auto_tool_search` + `current_time`. It demonstrates the loop: user
asks for a task it can't do → agent discovers a capability → attaches it →
completes the task on the next turn.
