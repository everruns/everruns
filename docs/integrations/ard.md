---
title: Agentic Resource Discovery (ARD) for Runtime Capability Attachment
description: Let agents discover and attach external MCP servers and A2A agents at runtime via the Agentic Resource Discovery protocol. Configure registries, trust gating, and SSRF-safe attachment.
sidebar:
  label: ARD Discovery
---

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="52.0" height="52.0" aria-hidden="true" style="float: right; margin-left: 16px;"><circle cx="12" cy="5" r="2.3" fill="none" stroke="currentColor" stroke-width="1.6"/><circle cx="5" cy="18" r="2.3" fill="none" stroke="currentColor" stroke-width="1.6"/><circle cx="19" cy="18" r="2.3" fill="none" stroke="currentColor" stroke-width="1.6"/><path d="M10.7 6.9L6.3 15.9M13.3 6.9L17.7 15.9M7.3 18h9.4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>

Everruns integrates with [Agentic Resource Discovery (ARD)](https://agenticresourcediscovery.org/spec/)
as a **client**: a running agent can search ARD registries for capabilities it
was not pre-provisioned with, MCP servers and A2A agents, and attach them to
its session on the fly. Newly attached MCP tools appear on the next turn;
attached A2A agents become `spawn_agent` targets.

ARD is the discovery layer *above* `tool_search`. `tool_search` defers schemas
for tools already attached to a session; ARD decides **which** MCP server / A2A
agent to attach in the first place.

> **Status:** Experimental (available in Dev environments).

## What You Get

- **Runtime discovery**: `discover_resources` runs a semantic search against a
  configured registry, outside the model context (like `tool_search`).
- **Dynamic attachment**: `attach_resource` materializes a result as a
  session-scoped MCP server or external A2A agent, reusing existing Everruns
  config-overlay machinery. The agent loop is unchanged.
- **Safety by construction**: registry allowlist, `trustManifest` verification,
  SSRF-safe URL validation, and a per-session attachment cap.

## Quick Start

### 1. Enable the capability on an agent

Add the `resource_discovery` capability and point it at one or more registries.
The model selects a registry by `id`, it can never supply a raw URL.

```json
{
  "registries": [
    { "id": "public", "url": "https://agenticresourcediscovery.org/api/v1", "federation": "none" }
  ],
  "require_trust": [],
  "allow_attach_types": ["application/mcp-server+json", "application/a2a-agent-card+json"],
  "max_attachments": 5,
  "allow_local_urls": false
}
```

The ready-made **Capability Scout** seed agent (Dev) ships with this wired to the
public reference registry plus `tool_search`.

### 2. (Optional) Connect a registry token

For registries that require authentication, connect **Agentic Resource
Discovery** under **Settings → Connections** (provider `ard`) and paste a bearer
token, or set the `ARD_REGISTRY_TOKEN` session secret. Public anonymous-read
registries need no token.

### 3. Discover and attach

From a session, ask for something the agent can't yet do. It will:

1. `discover_resources({ text: "..." })`, search the registry and get ranked
   candidates, each with a `urn`.
2. `attach_resource({ urn })`, verify trust, validate the URL, and attach.
3. Use the new capability on the next turn, MCP tools appear prefixed
   `mcp_<name>__*` (surfaced through `tool_search`); A2A agents are reachable via
   `spawn_agent`.
4. `list_attached_resources()`, see what's attached this session.

## Tools

| Tool | Description |
|---|---|
| `discover_resources({ text, filter?, registry_id? })` | Search a configured registry (`POST /search`). Returns ranked `{ urn, displayName, type, score, source, description, attachable }`. `registry_id` is optional when one registry is configured. |
| `attach_resource({ urn })` | Resolve a discovered entry, verify `trustManifest` + `require_trust`, SSRF-validate the URL, and attach it. Idempotent per URN. |
| `list_attached_resources()` | List attachments for the session (visibility / audit). |

## Attachment Lifecycle

- Attachments are **session-scoped** and torn down when the session ends.
- MCP entries become a session-scoped `mcpServers` record; their tools are then
  subject to `tool_search` deferral.
- A2A entries merge into the session's A2A delegation config and are driven
  through the existing `spawn_agent` / `wait_task` / `message_task` tools.
- Re-attaching the same URN is a no-op (reports `already_attached`).

## Security

- **Registry allowlist**: only configured registries are queryable; the model
  picks a `registry_id`, never a URL.
- **Trust gate**: an entry's `trustManifest` identity domain must match its URN
  publisher, and any `require_trust` attestations (e.g. `["soc2"]`) must be
  present, before it can be attached.
- **SSRF protection**: every resolved artifact and endpoint URL is validated
  (DNS-pinned; loopback, private, link-local, and cloud-metadata addresses are
  blocked). `allow_local_urls` relaxes this for local testing only.
- **Attachment cap**: `max_attachments` bounds how many capabilities a single
  session can attach, limiting prompt-injection-driven attach storms.
- **Untrusted data**: all registry-returned text is treated as untrusted
  external input.

See the co-located [`SPEC.md`](https://github.com/everruns/everruns/blob/main/crates/ard/SPEC.md)
for architecture and the full security review.
