---
type: Decision
title: "Agent MCP Attachments (acts-as semantics)"
description: "Make who an MCP server acts as an explicit, fail-closed property of an Agent attachment; org MCP servers become presets; one MCP surface per Agent."
tags:
  - everruns
  - integrations
  - agents
  - mcp
---
# Agent MCP Attachments (acts-as semantics)

> Status: **Proposed.** Supersedes nothing yet. [mcp-servers.md](mcp-servers.md) and
> [runtime-mcp.md](runtime-mcp.md) remain authoritative for what exists today.

## Abstract

Today an MCP server's identity semantics are a side effect of its `auth_mode`:
`api_key` happens to be org-wide, `oauth` happens to be per-user, and
`agent_identity_connections` silently shadows `user_connections` when a session
has an identity. Nothing in the model says *who the agent acts as* on a remote
system, so the answer differs per server, per session, and per whether a human
happened to be present.

This decision makes that the first-class property: every MCP attachment declares
`actsAs`, resolution is fail-closed on it, and the org MCP catalog becomes a set
of presets rather than a second place where auth lives.

## Problem

Four overlapping ways to reach an MCP server exist (`mcp-servers.md`):

1. Org `mcp_servers` rows, attached to an agent as capability `mcp:{uuid}`.
2. Scoped `mcpServers` maps on harness/agent/session.
3. Capability-contributed servers (`collect_capability_mcp_servers`).
4. Agent-bound tool-parameter credentials (`agent_mcp_secret_bindings`).

Each carries different, implicit identity semantics:

| Path | Whose identity today | Stated anywhere |
|---|---|---|
| Org server, `auth_mode = api_key` | organization | no |
| Org server, `auth_mode = oauth` | invoking user, unless the session has an identity, then the identity | no |
| Scoped `mcpServers` | none — explicit OAuth is stripped (`strip_untrusted_oauth_from_scoped_mcp_servers`) | in code only |
| Capability-contributed | whatever the contribution declares | no |
| Tool-parameter binding | organization + agent | yes |

Three consequences:

- **Silent identity substitution.** `DbConnectionResolver` prefers an identity
  connection and falls back to user connections. A change in session wiring
  moves which account a write lands under, with no configuration change and no
  event that says so.
- **No way to express "the agent itself".** The Linear use case wants issue
  updates authored by the Agent, uniformly for every teammate who invokes it.
  The only way to approximate that today is an org API key, which most modern
  MCP servers do not accept.
- **Unattended runs borrow a human.** A trigger-fired session resolves "the
  session's resolved owner user"; the code itself flags first-org-member-wins as
  a hazard for leased resources.

## Decision

### D1. `actsAs` is an explicit, required property of every attachment

```
actsAs ∈ { none, service, user }
```

- **`none`** — no credential is resolved. Literal headers only.
- **`service`** — the agent acts as *itself*. The grant is owned by the agent's
  `AgentIdentity` and shared by every session and every invoking user.
- **`user`** — the agent acts *on behalf of the invoking user*. The grant is the
  user's own.

`actsAs` is stored in the agent definition, not in a side table, so it travels
with agent versions, exports and the declarative blueprint like every other
agent property.

### D2. Resolution is fail-closed on `actsAs`, with no fallback

| `actsAs` | Reads | Never reads | Missing grant |
|---|---|---|---|
| `none` | literal headers | any connection store | n/a |
| `service` | `agent_identity_connections` for the agent's identity | `user_connections`, org `api_key_encrypted` of a user-bound preset | `connection_required` naming the **agent**, setup URL = agent MCP tab (admin action) |
| `user` | `user_connections` for the invoking user | `agent_identity_connections`, org API keys, catalog `Authorization` headers | `connection_required` naming the **user**, setup URL = their connections page |

The existing identity→user fallback in `DbConnectionResolver` is removed for MCP
resolution. It stays for the non-MCP providers that rely on it today, or is
retired with them separately; either way MCP no longer depends on it.

**A `user` attachment can never carry service auth** (requirement 2). Enforcement
generalizes the existing strip: for `actsAs = user` the resolver discards the
preset's `api_key`, discards any `Authorization` in preset or inline headers, and
resolves solely from the invoking user's connection. This is a resolver-side
invariant, not validation, so a stored config cannot drift past it.

**Unattended sessions can only use `service` attachments.** A `user` attachment
in a session with no human initiator returns `connection_required` rather than
resolving the owner user's token. This closes first-org-member-wins for MCP.

A session counts as having a human initiator when its *owning principal is
itself a `user` principal*. `sessions.resolved_owner_user_id` is deliberately
not the signal: it walks the principal parent chain, so a trigger-created
agent-identity principal parented to its creator still resolves to a human —
which is precisely the borrow this rule forbids.

### D3. Org MCP servers become presets

An org `mcp_servers` row is a **catalog entry**: name, URL, description,
`protocol_mode`, and — where the server speaks OAuth — the registered client
(`settings.oauth`, already present: issuer, endpoints, `client_id`,
`client_secret_encrypted`). It is a template, not an attachment.

An agent references a preset by name:

```json
{
  "mcpServers": {
    "linear":  { "use": "catalog:linear", "actsAs": "service" },
    "jira":    { "use": "catalog:jira",   "actsAs": "user" },
    "docs":    { "type": "http", "url": "https://example.com/mcp", "actsAs": "none" }
  }
}
```

- A preset supplies transport, URL, era policy and OAuth client registration.
  The attachment supplies the logical name (tool prefix) and `actsAs`.
- The same preset can be attached twice by two agents with different `actsAs`.
  The grants are separate rows in separate stores; nothing is shared but the
  client registration.
- A missing, archived or deleted preset resolves to a disabled attachment with a
  named error. Fail closed — never silently drop the server and run without it.
- **Org-held credentials on a preset are service semantics only.** An org API key
  is the organization acting, never the invoking user. Requirement 3 holds:
  org-level auth stays possible, with service meaning.

### D4. OAuth requires a preset; inline attachments stay credential-free

OAuth needs somewhere durable to keep the dynamically registered client
(RFC 7591) and the discovered endpoints (RFC 9728). That is the preset's
`settings.oauth`. So:

- `actsAs ∈ {service, user}` ⇒ the attachment must reference a preset.
- An inline (`type`/`url`) attachment is `actsAs: none`. This is today's
  `strip_untrusted_oauth_from_scoped_mcp_servers` behavior, now with a stated
  reason rather than as a special case.

"Add a custom server" therefore stays a two-field form, and "connect a server
that needs a login" is an org-admin action once, reused by every agent.

### D5. Capability-contributed servers keep the same semantics

`collect_capability_mcp_servers` continues to contribute servers, and each
contribution declares `actsAs` like any other attachment. Contributions are
trusted (built-in code or an installed plugin), so `service` and `user` are both
available to them and D4's preset requirement is relaxed: a contribution may
carry its own registered client. Contributed attachments are read-only in the
agent surface, tagged with their contributing capability, and still lose to
explicit agent and session layers by name (existing merge order).

### D6. Service attachments keep human attribution

`service` deliberately loses the invoking human on the remote side. Everruns
keeps it locally: sessions already record `initiator`, `acting_principal`,
`initiator_principal_id` and `acting_principal_id`
([agent-identities.md](../runtime-resources/agent-identities.md)). Every tool call
through a `service` attachment records the initiator in event provenance, so
"the Agent did it, because Alice asked" is answerable from the event log even
when Linear only shows the Agent.

### D7. Agent identity is created when the first service attachment is authorized

Today an `AgentIdentity` appears lazily on first unattended action. A service
grant needs an owner at authorize time, so authorizing a `service` attachment
creates the identity if absent, using the same idempotent guarded write. No
change to the lazy path for agents that never use service MCP.

### D8. Tool-list caching is keyed by identity scope

The existing `cacheScope` rule (public shared, private keyed by credential hash)
extends to the attachment:

- `none` — cacheable org-wide, as today.
- `service` — cacheable per (preset, agent): the grant is shared by every caller
  of that agent, so a persisted cache is sound. This is new capability; today
  OAuth servers are excluded from caching entirely and pay a live `tools/list`
  per run.
- `user` — per (preset, user), short TTL, in memory, never written to the org
  row's `cached_tools`.

The 24h max-stale bound and the exclusion of revoked grants are unchanged.

## SOTA

Where the industry has landed, and what it implies. Treat as an orientation
survey, not a citation list.

- **Admin-configured catalog, per-member authorization** is the common baseline
  (Claude's org connectors, ChatGPT/AgentKit connectors): an admin registers the
  connector once, each member authorizes their own grant. That is exactly D3 +
  `actsAs: user`. It is the right default for assistant-shaped products where
  every action should carry the human's name.
- **App-as-actor is the differentiator for unattended work.** Linear's OAuth
  supports an application actor so the app authors issues and comments as
  itself; Warp's factories use the team's managed integration with the app as
  the API actor and track the requester separately for attribution. Products
  that run agents on a schedule converge on this, because a per-user grant makes
  scheduled work depend on one employee's token and disappear when they leave.
- **The failure mode everyone hits is ambiguity, not capability.** The screenshot
  that prompted this work is a user asking which identity their agent acts under
  — in a product that supports both. The lesson is that the answer must be on
  the attachment, visible in the UI, and not inferable from auth mechanics.
- **Transport and auth mechanics are settled and we already implement them:**
  RFC 9728 protected-resource discovery, RFC 7591 dynamic registration, PKCE,
  RFC 8707 resource-bound tokens (`crates/mcp/src/oauth/`). Nothing in this
  decision needs new protocol work — it needs the grants to land in the right
  store and the UI to say which one.

Net: SOTA supports both modes; nobody names them well. Naming them is the
product decision here.

## UI

One rule: **an attachment is a row, not a screen.**

### Agent → MCP tab

A new `SectionTabs` entry on the agent detail page, beside Overview/Sessions. One
list, one row per effective attachment, in merge order:

```
linear   Catalog · Linear         Acts as: Agent       14 tools   ● Connected
jira     Catalog · Jira           Acts as: Each user    8 tools   ○ You: not connected
docs     Inline                   Acts as: —            3 tools   ● Reachable
slack    From capability "Slack"  Acts as: Agent       21 tools   ● Connected
```

- Source chip and acts-as chip are the two facts that resolve the Warp question
  at a glance. Both are plain text, no iconography to decode.
- Right-aligned action per state: **Authorize** (service, unauthorized, admin
  only), **Connect** (user, and it is *your* grant missing), **Connected as
  `<handle>`** (read-only, with revoke in the row menu), nothing for capability
  rows beyond a link to the capability.
- Tool count expands inline to the tool list. No detail page.

### Adding

One popover from **Add MCP server**: a search field over org presets, each a
single line (name, host, auth kind). Picking one asks exactly one question, two
radio rows:

```
Who does this act as?
  ( ) The agent        one login, shared by everyone who uses this agent
  ( ) Each user        every teammate connects their own account
```

Preset + choice = attachment. Two clicks and a radio. **Add custom…** at the
bottom of the list opens the existing three-field inline form (name, URL,
headers) with acts-as fixed to none and a one-line explanation of why.

### MCP catalog page (`/mcp-servers`, nav label "MCP")

Stays the org admin surface, gains what makes it a catalog rather than a list:

- Columns: name, host, transport/era, auth kind, **used by N agents**, status.
- Row click opens the existing edit dialog; no new screens.
- A second tab, **My connections**: the MCP grants *you* personally hold, with
  revoke. This is the only place a user can answer "what have I authorized?",
  which today has no surface at all.

### Chat-time

Unchanged. The `connection_required` structured result and the URL-elicitation
consent card ([mcp-servers.md](mcp-servers.md)) already handle the missing-grant
path; they gain an accurate subject line because the resolver now knows whether
the agent or the user is missing a grant.

## Phasing

| Phase | Scope |
|---|---|
| 0 | `actsAs` on `ScopedMcpServer` + `use: catalog:<name>` references; validation; no behavior change (existing configs resolve to today's semantics) |
| 1 | Fail-closed resolution; service grants in `agent_identity_connections`; eager identity on authorize; unattended runs restricted to `service` — **resolution and the unattended restriction landed** (EVE-1029); authorizing service grants and eager identity on authorize remain open (EVE-1030) |
| 2 | Linear preset end to end, with the application actor; first vertical proof |
| 3 | Agent MCP tab and the add popover |
| 4 | Catalog page columns, My connections tab |
| 5 | Capability contributions declare `actsAs`; identity-scoped tool-list caching |

Phases 0–2 are backend-only and independently shippable. Phase 3 is the first
user-visible change.

## Migration

- Existing org servers with `auth_mode = api_key` → preset, attachments default
  `actsAs: service`. Same behavior, now stated.
- Existing org servers with `auth_mode = oauth` → preset, attachments default
  `actsAs: user`. Same behavior for installs that rely on per-user grants;
  no silent promotion to service.
- Existing scoped `mcpServers` entries → `actsAs: none`. That is already what
  the strip produces.
- Sessions that today resolve through an identity connection change behavior
  only when the attachment is explicitly set to `service`. The switch is a
  config edit, visible in the agent version diff.

## Open questions

1. **Who may authorize a service grant?** Proposal: `OrgMcpServersManage`, the
   permission that already gates preset creation. An agent editor without it
   sees the attachment and an explanatory "ask an admin" state.
2. **Does a `service` grant survive agent deletion?** The identity outlives the
   agent today. Proposal: revoke on agent delete, since the grant was
   authorized for that agent's work.
3. **Per-channel override.** Should an agent channel (Slack, A2A) be able to
   force `user` on an attachment the agent declares as `service`? Deferred —
   real requirement, but it needs the channel-scoped config layer that
   [agent-exposure.md](agent-exposure.md) is still building.
