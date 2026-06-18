---
title: Resource Discovery (ARD)
description: Let agents discover and attach external MCP servers and A2A agents from operator-configured Agentic Resource Discovery (ARD) registries, with trust and SSRF controls.
---

Everruns can act as a client of [Agentic Resource Discovery (ARD)](https://agenticresourcediscovery.org/spec/)
registries. The experimental `resource_discovery` capability lets agents search a
registry for external capabilities and attach the trusted ones into the current
session — MCP servers become tool providers and A2A agent cards become
delegation targets.

> Experimental: available on Dev-grade deployments only. The capability may change.

## What You Get

- **Discovery**: search operator-configured registries by natural-language intent.
- **Attach**: resolve a candidate by URN, verify its trust manifest, validate its endpoint against SSRF rules, and record it as a session resource.
- **Visibility**: list what has been attached to the session.

## Security model

- The model selects a registry by configured `registry_id` — it can **never** supply a raw registry or resource URL.
- Every resolved endpoint URL is SSRF-validated (loopback, RFC1918, link-local, cloud-metadata and related ranges are blocked).
- `require_trust` (default on) requires a trust manifest whose identity matches the resource URN's domain before any attach.
- `max_attachments` caps how many resources a session may attach.
- All registry responses are treated as untrusted data.

## Quick Start

### 1. Configure registries

Add the `resource_discovery` capability to an agent and configure at least one
registry:

```json
{
  "registries": [
    { "id": "main", "url": "https://ard.example.com" }
  ],
  "require_trust": true,
  "allow_attach_types": ["mcp_server", "a2a_agent"],
  "max_attachments": 8,
  "allow_local_urls": false
}
```

- `registries[].id` — the handle the model uses; `registries[].url` — operator-controlled base URL; `registries[].federation` — optional allowlist of registry hostnames this registry may delegate to.
- Set `allow_local_urls: true` only for local testing; it bypasses local-address blocking and nothing else.

### 2. Use in sessions

| Tool | Description |
|------|-------------|
| `discover_resources` | Search a registry for external capabilities. |
| `attach_resource` | Resolve, verify, and attach a discovered resource by URN. |
| `list_attached_resources` | List resources attached to the session. |

#### `discover_resources`

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `text` | string | Yes | What capability you need. |
| `filter` | object | No | Structured filter forwarded to the registry. |
| `registry_id` | string | No | Which configured registry to search (required if more than one). |

Returns ranked `{ urn, displayName, type, score, source, description }` hits.

#### `attach_resource`

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `urn` | string | Yes | URN of a resource from `discover_resources`. |
| `registry_id` | string | No | Registry that surfaced the URN (required if more than one). |

Resolves the entry, enforces value-or-reference, maps its media type
(`application/mcp-server+json` → MCP server, `application/a2a-agent-card+json` →
external A2A agent), runs the trust and SSRF gates, enforces `max_attachments`,
and records the attachment. Idempotent per URN.

## Lifecycle

1. `discover_resources` returns ranked candidates by URN.
2. `attach_resource` verifies and records the attachment as a session resource
   (`mcp_server` / `external_a2a_agent`). For an **MCP server**, it also persists
   the validated endpoint into the session's MCP overlay so the server's tools
   become callable on the **next turn**; the result includes
   `callable_next_turn: true`. **A2A agent** attaches are visibility-only for now
   and report `callable_next_turn: false`.
3. `list_attached_resources` shows current attachments.

MCP-server attachments take effect on the next turn (the server re-resolves the
session's MCP overlay each turn). Making a freshly attached **A2A agent**
callable within the same session requires an additional runtime seam that is a
planned follow-up; see the crate `SPEC.md`.

## Security

See the crate [`SPEC.md`](https://github.com/everruns/everruns/blob/main/integrations/ard/SPEC.md)
security-review table and the ARD section of the threat model for the full
control set (registry allowlist, trust gate, SSRF validation, `max_attachments`,
untrusted-data handling).
