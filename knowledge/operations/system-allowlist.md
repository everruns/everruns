---
type: Specification
title: "System-wide Outbound Allowlist"
description: "System-wide outbound allowlist (\"green list\")."
tags:
  - everruns
  - operations
---
# System-wide Outbound Allowlist

## Intent

An optional, host-owned global allowlist ("green list") of well-known public
resources that the egress boundary permits for tenant/agent-directed outbound
traffic. It is a deployment-wide safety net that constrains capability, MCP,
integration, and generic runtime HTTP traffic to a curated set of trusted public
services, independently of per-agent/session `NetworkAccessList`.

Unlike `NetworkAccessList`, this is **not** end-user or agent configuration. It
is an internal, maintainer-curated list shipped with the binary. Operators turn
it on or off; they do not edit it per deployment.

## Data Model

Source of truth is an embedded TOML file, `crates/core/src/system_allowlist.toml`,
organized into named groups so it stays manageable instead of one flat list:

```toml
[groups.package_registries]
description = "Language and package manager registries (npm, crates, pypi, ...)."
allowed = ["*.npmjs.org", "*.crates.io", "*.pypi.org", ...]

[groups.ai_providers]
description = "LLM and AI provider APIs."
allowed = ["*.openai.com", "*.anthropic.com", "*.googleapis.com", ...]
```

Each group has an optional `description` and a list of `allowed` host patterns.
Patterns use the same format and matching rules as `NetworkAccessList` (see
`knowledge/operations/network-access.md`):

- `example.com` — exact domain
- `*.example.com` — domain and all subdomains (apex included)
- `https://example.com/api/` — URL prefix

Current groups: `package_registries`, `source_hosting`, `container_registries`,
`ai_providers`, `cloud_providers`, `os_packages`, `developer_tools`.

`SystemAllowlist` flattens all group patterns into a single non-empty `allowed`
`NetworkAccessList`, so only URLs matching at least one pattern are permitted.
An allowlist with no patterns (empty/misconfigured TOML) **fails closed** — it
denies every URL rather than allowing all — via a sentinel pattern, since an
empty `NetworkAccessList.allowed` otherwise means "no restriction". See
`crates/core/src/system_allowlist.rs`.

## Enabling

Disabled by default. Controlled by a single environment variable:

```
EVERRUNS_SYSTEM_ALLOWLIST_ENABLED=true   # or 1
```

`DirectEgressService::for_runtime_traffic_from_env()` resolves the allowlist at
construction. When unset or falsy, egress behavior is unchanged. Host-owned
system transports do not construct or call `EgressService`, so they do not read
this toggle.

The env var is read by every process that builds an egress service, so it
applies uniformly across the **control plane** and **workers**:

- `crates/server/src/platform.rs` — control-plane / in-process platform.
- `crates/worker/src/platform.rs` — distributed worker platform.
- `crates/server/src/domains/mcp_servers/service.rs` — MCP server egress.

Each runtime/agent egress surface must construct egress via
`DirectEgressService::for_runtime_traffic_from_env()` (not `::default()`) so the
toggle is honored everywhere. The list contents are *not* env-configurable —
they are the curated embedded TOML; only the on/off toggle is environmental.

## Enforcement

The `EgressService` is the tenant/agent runtime outbound boundary (see
`knowledge/operations/egress.md`). When the system allowlist is active, `DirectEgressService`
denies tenant/agent-directed request kinds whose URL does not match the
allowlist, with `EgressError::NetworkAccessDenied`.

This check applies to `capability`, `integration`, `mcp`, and generic `other`
requests, independent of the per-request `network_access`. Both the system
allowlist (if active) and the per-request `NetworkAccessList` (if present) must
permit a URL for those requests to proceed.

Host-owned system transports are intentionally outside the tenant/agent policy:
system email, utility LLM, LLM provider/model-discovery transports, Daytona
provider APIs, and similar fixed deployment-owned clients are configured by the
deployment, keep their credentials in platform services, and do not route
through `EgressService`. They must not require adding provider endpoints such as
`api.resend.com` to the tenant/agent allowlist.

### Maximum priority (hard ceiling)

The system allowlist is a separate, AND-ed gate — it is **never merged into**
the harness/agent/session `NetworkAccessList`. Those layers can only narrow
within the system allowlist (intersection on `allowed`, union on `blocked`);
they can **never widen past it or override it**. When the allowlist is enabled,
a session that explicitly allows a host still cannot reach it through
tenant/agent egress unless the system allowlist also lists it. The system
allowlist always wins for the request kinds it governs.

### fetchkit / web_fetch

When `ToolContext.egress_service` is present (always true in the runtime),
`web_fetch` injects the egress boundary as fetchkit's HTTP transport
(`integrations/web-fetch/src/egress_transport.rs`), so the allowlist is
enforced at the boundary for every hop like any other egress traffic.

On both paths the tool pre-checks the initial URL and returns the distinct
"Endpoint blocked by system policy: …" error before any request is made
(`integrations/web-fetch/src/lib.rs`). A denial raised at the egress
boundary itself (e.g. a redirect hop) surfaces as "Outbound request blocked by
network policy: …". On the direct path (contexts without an egress service,
e.g. embedded hosts) the pre-flight check is the only enforcement.

### Operator responsibility

Because enforcement covers tenant/agent runtime traffic, an operator enabling
the allowlist must ensure endpoints reachable through capabilities,
integrations, MCP, plugin fetches, and similar runtime HTTP paths are covered by
a group. Self-hosted or uncommon runtime endpoints not present in the curated
groups will be blocked while the allowlist is enabled. Host-owned system
transports such as email, utility LLM, LLM providers, and Daytona are configured
separately by deployment environment and are not governed by this list.

## Relationship to other controls

| Control | Scope | Configured by |
|---------|-------|---------------|
| System allowlist | Tenant/agent runtime egress | Maintainers (curated), operator toggles via env |
| `NetworkAccessList` | Per harness/agent/session, agent-authored URLs | Users/agents |
| Future Egress Gateway | Network component owning outbound policy | Deployment |

The system allowlist is a precursor to the Egress Gateway's outbound allowlist
described in `knowledge/operations/egress.md`: it gives a single deployment-wide allowlist
today, in-process, ahead of the remote gateway.

## Threat Model

Reinforces **TM-AGENT-018** (outbound URL filtering) with a deployment-wide
backstop that does not depend on per-agent configuration being set correctly,
and narrows the blast radius of SSRF or exfiltration attempts to a curated set
of public services.
