# everruns-ard

Agentic Resource Discovery (ARD) **client** capability for Everruns agents.

ARD is a platform-level discovery protocol, a sibling of MCP and A2A, not an
external-service integration, so it lives in `crates/` alongside `everruns-mcp`.

Adds a `resource_discovery` capability that lets a running agent discover
external capabilities (MCP servers, A2A agents) from ARD registries and attach
them mid-session. This is the consumer side of the
[ARD protocol](https://agenticresourcediscovery.org/spec/); publishing an
Everruns catalog/registry (ARD-as-a-server) is tracked separately.

It is the discovery layer *above* `tool_search`: `tool_search` defers schemas
for tools already attached to a session; ARD decides *which* MCP server / A2A
agent to attach in the first place.

## Tools

- `discover_resources({ text, filter?, registry_id? })`, semantic `POST /search`
  against a configured registry; returns ranked entries with a `urn`.
- `attach_resource({ urn })`, verify trust + SSRF, then materialize the entry as
  a session-scoped MCP server or external A2A agent. Idempotent per URN.
- `list_attached_resources()`, list what's attached this session.

## Configuration

The capability config carries a registry **allowlist** (`registries[]`), a trust
gate (`require_trust`), an attachment type allowlist (`allow_attach_types`), an
attach cap (`max_attachments`), and a local-URL escape hatch (`allow_local_urls`,
tests/dev only). The model selects a `registry_id`; raw URLs are never accepted.

Auth is connection-backed: a registry bearer token via the `ard` connection
provider, falling back to the `ARD_REGISTRY_TOKEN` session secret/env var.
Anonymous-read registries need no token.

See [`SPEC.md`](SPEC.md) for architecture, attachment lifecycle, and the security
review, and `docs/integrations/ard.md` for a quick start.

## Status

Experimental (Dev only).
