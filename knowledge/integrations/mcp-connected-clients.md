---
type: Proposal
title: "Connected AI Clients"
description: "See, revoke, attribute and limit the external MCP clients (Claude, ChatGPT, Cursor, Grok) a user approved to act as them on /mcp."
tags:
  - everruns
  - integrations
  - mcp
  - security
---
# Connected AI Clients

> Status: **Accepted** (2026-10-09). Phase 1 built (grants, revoke with the cached `/mcp`
> check, `last_used_at`, Connected AI clients panel). Phase 2 now records inbound
> client provenance and clarifies consent; phase 3 permission selection remains separate.

## The problem

A user connects Claude, ChatGPT, Cursor or Grok to `{root}/mcp`. The client
registers itself (RFC 7591), the user approves it once on `/oauth/authorize`,
and from then on the client acts as that user in every organization they
belong to. After that:

- **The user can't see it.** No screen lists the clients they approved.
  Settings > My agent experience covers the other direction only (MCP servers
  that Everruns agents call as the user, the My MCP servers list).
- **The user can't take it back.** Refresh tokens live 30 days
  (`MCP_REFRESH_TOKEN_LIFETIME_SECS` in `crates/server/src/auth/mcp_oauth/mod.rs`).
  The only cut-off is deleting the user.
- **Nobody can tell which client did something.** The `mcp_access` JWT carries
  user, roles and audience, but no client id (`generate_mcp_access_token` in
  `crates/server/src/auth/jwt.rs`). Change history records the MCP surface
  (`ChangeSurface::Mcp`) but not which client.
- **There is one level of access: everything.** The only scope is `mcp`, and
  the consent page says so: call any tool in any organization, as the user.

Products that let AI clients act for a person (Shipmail's Connections page, for
example) show each client with its owner, its permissions and when it was last
used, and let the person change or revoke it.

## What exists to build on

| Piece | Where | Use |
|---|---|---|
| `oauth_clients` (client name, redirect URIs) | `crates/server/migrations/legacy/009_v0.8.8.sql` | client identity |
| `oauth_refresh_tokens` (user, org, client, expiry) | same | today's only trace of an approval |
| Consent page | `oauth_authorize` in `crates/server/src/auth/mcp_oauth/mod.rs` | where permissions get chosen |
| Command `read_only()` | `crates/server/src/domains/common/mod.rs` | what read-only access may run |
| `ChangeIntent` | `crates/server/src/domains/change_history/` | where "via Cursor" gets recorded |
| `McpResolvedOrg`, per-call `organization_id` | `crates/server/src/api/mcp_endpoint/` | where an org limit is enforced |

## Proposal

### 1. A grant per approval

A new `oauth_grants` row is written when the user approves a client: client id,
user, access level, allowed orgs (none means all), `created_at`,
`last_used_at`, `revoked_at`. Re-approving the same client updates the same
grant instead of stacking a second one. Re-approving after a revoke replaces
the revoked row with a fresh grant id, so tokens that named the revoked grant
stay rejected. Refresh tokens point at their grant, and revoking a grant
deletes them.

Existing refresh tokens are backfilled into grants with full access and all
orgs, so nothing already connected stops working.

### 2. Tokens name their grant

`mcp_access` tokens gain `client_id` and `grant_id` claims. `/mcp` checks the
grant on each request through a short-lived cache, so a revoke takes effect
within about 30 seconds on every replica, not when the token expires. If the
lookup fails, the request is rejected. The same check updates `last_used_at`,
at most every few minutes per grant, so the list stays cheap to keep current.

### 3. Attribution

The authenticated server resolves the registered client only after checking
its audience-bound token and live grant. Registration names remain self-declared;
neither a name, a redirect domain, nor MCP `clientInfo` verifies a publisher.
Consent identifies the selected callback host and explains that a localhost
callback reaches software on the device without authenticating that software.

A session keeps its creator separately from each input message's client. The
input event identifies the start, steering, or resume of a turn; a later client
or the web UI never inherits the creator's identity. Background execution does
not retain ambient inbound request attribution. Legacy tokens, PATs and sessions
without recorded provenance stay unattributed rather than guessing from names.
Forks retain copied message history but record their own creator.

The immutable creator column is set inside the session creation service. Input
provenance lives in server-owned event metadata (`mcp_client`), never in caller-owned
message metadata. Lookalike legacy message keys remain untrusted. Command contexts
carry attribution through scripted MCP execution, and entity history and audit
record it alongside the human actor. Public conversation projections expose only
the reviewed client and grant identifiers and the self-declared name. No bearer
token, refresh token, secret, or callback query is copied into provenance.

Implementation: [request context](../../crates/server/src/auth/mcp_attribution.rs),
[message intake](../../crates/server/src/domains/messages/service/mod.rs), and
[session creation](../../crates/server/src/domains/sessions/service/create.rs).

### 4. Permissions (planned phase 3)

The planned consent and Manage UI will offer two choices:

| Choice | Options | Enforced at |
|---|---|---|
| Access | **Read only**: `query`, `discover`, `me`, read-only commands, `resources/*`. **Read and run**: everything, including `execute` mutations, `agent_run` and `session_send_message` | MCP tool dispatch, using the command's `read_only()` flag. `agent_run` and messaging count as writes because they spend money and act |
| Organizations | All my organizations, or selected ones | `McpResolvedOrg` and the per-call `organization_id` override |

After phase 3, a client may request `mcp:read` instead of `mcp`. The consent page will start
from what the client asked for, and the user can only lower it. Personal access
tokens keep their own page and full access. Giving them the same choices is a
follow-up.

### 5. UI

**Connected AI clients** is **Settings > AI clients**. Each row shows:

- the client name and icon (from the registered `logo_uri` when given, else
  its initial)
- the host it redirects to, which tells "Cursor" apart from a lookalike
- when it was approved and when it was last used
- access level and organizations, shown as badges
- **Manage**, to change access or organizations
- **Revoke**, which confirms and then cuts the client off immediately

Registration starts accepting the optional RFC 7591 `logo_uri` and
`client_uri` (HTTPS only, never fetched by the server, rendered as an image
with a fallback) so rows aren't all letters.

Org admins see nothing new in this proposal. An admin view of the clients
acting in their organization is a natural follow-up once grants exist.

## Phases

1. Grants, the client and grant claims in the token, revoke with the
   cached check, `last_used_at`, and the list and Revoke in the UI.
2. Attribution in change history and the audit log.
3. Permissions on the consent page and in Manage, plus `mcp:read`.

Each phase is one PR and ships on its own.

## Threats

| Threat | Mitigation |
|---|---|
| A revoked client keeps working until its token expires | Per-request grant check, cache of about 30 seconds, rejected if the lookup fails |
| A lookalike client name ("Cursor") | The list and consent page show the redirect host next to the self-declared name |
| `logo_uri` used for tracking or SSRF | HTTPS only, the server never fetches it, and the UI loads it with `referrerpolicy=no-referrer` |
| A read-only grant escalates through a command that writes | Read-only means `read_only()` commands only, so a command that does not declare it is denied |

## Decisions

- The consent page defaults to **Read and run**, today's behaviour. Read-only
  clients can't do most of what people connect them for; the user can lower it
  on the consent page or later in Manage.
- A new grant covers **all organizations** by default, with "selected
  organizations" as an option.

## Remaining authorization work

Current inbound OAuth supports only full `mcp` access across the user's current
memberships. Unsupported scopes, including `mcp:read`, are rejected before consent
or approval; blank scope retains the existing `mcp` default. Consent and token
responses describe effective authority, not arbitrary requested strings.

Permission selection remains phase 3. The existing PAT read-only gate and domain
command policies are useful building blocks, but OAuth grant enforcement must also
cover organization overrides, tasks, resources, events, interactive replies, and
refresh/reapproval without widening a narrowed grant. Exposing a selector before
those checks exist would misrepresent consent.

Inbound CIMD remains a separate authorization-server change. Outbound support in
[runtime MCP](runtime-mcp.md) does not enable it. Before fetching an inbound client
metadata URL, define allowed origins and implement pinned egress, redirect and
size limits, client-id/document matching, bounded cache behavior, and metadata
change handling. The current DCR flow continues without adding that SSRF surface.
