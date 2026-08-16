---
type: Specification
title: "Network Access List"
description: "Network access allowlist/blocklist."
tags:
  - everruns
  - operations
---
# Network Access List

Controls which hosts/URLs an agent session can reach via network-capable tools
(web_fetch, future bashkit HTTP).

## Data Model

```typescript
interface NetworkAccessList {
  allowed?: string[];   // if non-empty, only matching URLs permitted
  blocked?: string[];   // always denied (takes precedence over allowed)
}
```

**Pattern format:**
- `example.com`, exact domain match
- `*.example.com`, domain and all subdomains
- `https://example.com/api/`, URL prefix match (scheme + host + path)

Matching is case-insensitive for domains. Blocked takes precedence over allowed.

## Layer Model

`NetworkAccessList` is a top-level field on **Harness**, **Agent**, and **Session**.
Not per-capability config, it's a cross-cutting security concern.

### Merge Semantics (each layer can only narrow, never widen)

| Field | Merge Rule | Rationale |
|-------|-----------|-----------|
| `allowed` | **Intersection**: child entries kept only if they match a parent pattern | Child cannot grant access parent didn't allow |
| `blocked` | **Union**: all blocked patterns from all layers combined | Child cannot un-block a parent's block |

If no layer sets `allowed`, all hosts are permitted (open by default).
If a child's `allowed` list is empty, it inherits the parent's list.

### Resolution Order

```
Harness (baseline)
  ∩ Agent (can only narrow)
    ∩ Session (can only narrow further)
```

Merge function: `network_access::merge_network_access(parent, child)`
- See `crates/core/src/network_access.rs` for implementation.

## Enforcement

All outbound HTTP/API call paths must use the host `EgressService` described in
`knowledge/operations/egress.md`. The egress service is the final enforcement point for
requests that carry a `NetworkAccessList`.

Merged `NetworkAccessList` flows through:
1. `ReasonAtom` merges harness + agent + session → stores on `RuntimeAgent.network_access`
2. `ReasonResult.network_access` carries it to `ActInput`
3. `ActAtom` sets `ToolContext.network_access`
4. Tools that send agent-authored or capability-configured URLs pass the list
   to `EgressRequest.network_access`
5. `EgressService` denies blocked/disallowed URLs before making the outbound
   request (THREAT[TM-AGENT-018])

`web_fetch` follows this path (`integrations/web-fetch/src/egress_transport.rs`),
re-checking the list on every redirect hop. Tools may additionally pre-check the
requested URL for a clearer user-facing error; the egress boundary remains the
final enforcement point.

### Bashkit

Outbound HTTP for `bashkit_shell` is opt-in via per-capability config
`{"enable_http": true}` and follows the same path as `web_fetch`: bashkit's
`HttpTransport` is backed by `EgressService`
(`integrations/bashkit/src/egress_transport.rs`), which
receives `ToolContext.network_access` and enforces it on every hop,
curl/wget follow redirects manually, so redirect targets are re-checked too.
With the config flag off (the default) or no egress service in context, the
interpreter has no network path at all (TM-BASH-003).

## API

All three resources accept `network_access` in create/update requests:

```json
// POST /v1/agents
{
  "name": "My Agent",
  "network_access": {
    "allowed": ["api.example.com", "*.github.com"],
    "blocked": ["evil.com"]
  }
}
```

```json
// POST /v1/sessions
{
  "agent_id": "agent_...",
  "network_access": {
    "blocked": ["internal.corp"]
  }
}
```

Setting `network_access` to `{}` (empty object) clears restrictions from that layer.
Omitting the field in update requests leaves it unchanged.

## Database

JSONB column `network_access` on `harnesses`, `agents`, `sessions` tables.
Migration: `010_v0.8.9.sql`.

## Threat Model

Mitigates **TM-AGENT-018** (no outbound URL filtering on web_fetch).
Per-agent/harness allowlist of permitted outbound domains, with blocked
patterns always denied.
