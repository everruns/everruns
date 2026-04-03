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
- `example.com` — exact domain match
- `*.example.com` — domain and all subdomains
- `https://example.com/api/` — URL prefix match (scheme + host + path)

Matching is case-insensitive for domains. Blocked takes precedence over allowed.

## Layer Model

`NetworkAccessList` is a top-level field on **Harness**, **Agent**, and **Session**.
Not per-capability config — it's a cross-cutting security concern.

### Merge Semantics (each layer can only narrow, never widen)

| Field | Merge Rule | Rationale |
|-------|-----------|-----------|
| `allowed` | **Intersection** — child entries kept only if they match a parent pattern | Child cannot grant access parent didn't allow |
| `blocked` | **Union** — all blocked patterns from all layers combined | Child cannot un-block a parent's block |

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

Merged `NetworkAccessList` flows through:
1. `ReasonAtom` merges harness + agent + session → stores on `RuntimeAgent.network_access`
2. `ReasonResult.network_access` carries it to `ActInput`
3. `ActAtom` sets `ToolContext.network_access`
4. `WebFetchTool.execute_with_context()` checks URL before fetching (THREAT[TM-AGENT-018])

### Bashkit

Bashkit has no network builtins (TM-BASH-003: curl/wget not available).
No enforcement needed today. If bashkit gains HTTP support, it must check
`ToolContext.network_access`.

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
Migration: `013_network_access.sql`.

## Threat Model

Mitigates **TM-AGENT-018** (no outbound URL filtering on web_fetch).
Per-agent/harness allowlist of permitted outbound domains, with blocked
patterns always denied.
