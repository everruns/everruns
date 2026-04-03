---
title: Network Access Control
description: Control which hosts and URLs agents can reach using layered allowlists and blocklists on harnesses, agents, and sessions
sidebar:
  order: 30
---

By default, agents with the `web_fetch` capability can access any public URL. **Network access lists** let you restrict which hosts and URLs an agent can reach, giving you fine-grained control over outbound network access.

Access lists are configured at three levels — harness, agent, and session — and each layer can only make access *more* restrictive, never less.

## Configuration

A network access list has two fields:

| Field | Purpose | Default |
|-------|---------|---------|
| `allowed` | If non-empty, only matching URLs are permitted | `[]` (no restriction) |
| `blocked` | Always denied, even if matched by `allowed` | `[]` |

### Pattern Format

Patterns support three formats:

| Format | Example | Matches |
|--------|---------|---------|
| Exact domain | `api.example.com` | Any URL on that exact domain |
| Wildcard domain | `*.example.com` | The domain and all subdomains |
| URL prefix | `https://api.example.com/v1/` | URLs starting with that prefix |

Domain matching is case-insensitive. Blocked patterns always take precedence over allowed patterns.

### API Examples

Set a network access list when creating or updating an agent:

```json
POST /v1/agents
{
  "name": "Research Agent",
  "system_prompt": "You are a research assistant.",
  "network_access": {
    "allowed": ["*.github.com", "api.openai.com"],
    "blocked": ["evil.example.com"]
  }
}
```

Restrict a specific session further:

```json
POST /v1/sessions
{
  "agent_id": "agent_...",
  "network_access": {
    "blocked": ["internal.corp"]
  }
}
```

To clear a network access list on update, send an empty object:

```json
PATCH /v1/agents/{id}
{
  "network_access": {}
}
```

## Layer Merging

Network access lists merge across three layers using restrictive semantics — each layer can only narrow what the parent allows:

```
Harness (baseline)
  ∩ Agent (can only restrict further)
    ∩ Session (can only restrict further)
```

The merge rules:

| Field | Rule | Rationale |
|-------|------|-----------|
| `allowed` | **Intersection** — child entries kept only if covered by a parent pattern | Child cannot grant access the parent didn't allow |
| `blocked` | **Union** — all blocked patterns combined | Child cannot un-block a parent's block |

If a child layer doesn't set `allowed`, it inherits the parent's list unchanged.

### Example

Given this configuration:

```
Harness:  allowed: ["*.github.com", "*.openai.com"]
Agent:    allowed: ["api.github.com"], blocked: ["evil.com"]
Session:  blocked: ["malware.github.com"]
```

The effective policy for the session is:

- **Allowed**: `["api.github.com"]` — kept because it's a subset of `*.github.com`; `*.openai.com` dropped because the agent didn't include it
- **Blocked**: `["evil.com", "malware.github.com"]` — union of all layers

Only `api.github.com` is reachable, and both `evil.com` and `malware.github.com` are explicitly denied.

### Harness Inheritance

If a harness inherits from a parent harness (via `parent_harness_id`), the network access list is merged through the inheritance chain before any agent or session layer is applied. The same intersection/union rules apply.

## Enforcement

The merged network access list is checked in `web_fetch` before every HTTP request. If a URL doesn't match the effective policy, the tool returns an error:

```
URL blocked by network access policy: https://blocked-domain.com/path
```

Standard SSRF protections (blocking private IPs, loopback, cloud metadata endpoints) are always enforced regardless of the network access list.

## Capabilities Affected

| Capability | Enforced | Notes |
|-----------|----------|-------|
| `web_fetch` | Yes | Checked before every HTTP request |
| `virtual_bash` | N/A | No network builtins (curl/wget not available) |
