---
type: Specification
title: "User Connections"
description: "User-scoped links to external accounts, with credentials resolved lazily at tool-execution time and never exposed to the model."
tags:
  - everruns
  - integrations
  - connections
  - security
---
# User Connections

## Abstract

A user connection links an external service account (GitHub, GitLab, Bitbucket,
Daytona, MCP servers, and other connector-registered providers) to an Everruns
user, independent of how that user authenticates. Someone who signed in with
Google can still connect GitHub for repository access or a Daytona API key for
sandboxes. Tokens are resolved lazily at tool-execution time, so tools such as
`git_clone` act as the user without the credential ever entering the model's
context.

## Design Decisions

- **Connections are decoupled from the login identity.** Authentication answers
  "who is this", a connection answers "what may this person's agent reach".
  Binding the two would make repository access a property of the login provider,
  which is wrong for every user who signed up by email.
- **Connections are user-scoped, not organization-scoped.** An installation
  represents that person's grant of access to their own repositories; different
  members of one organization legitimately reach different repositories.
  Connections are therefore also private: the list endpoint returns only the
  caller's own, and no organization role can enumerate another member's.
- **Resolution is scoped to the session's resolved owner, and never falls
  back sideways.** A session may use an `agent_identity_connection` attached to
  its `agent_identity_id`, or a `user_connection` owned by
  `sessions.resolved_owner_user_id` — never another member's connection. If the
  resolved owner has not connected the provider, the tool returns
  `connection_required` guidance pointing at Settings rather than borrowing
  someone else's access. See [Agent Identities](../runtime-resources/agent-identities.md).
- **Tokens are resolved lazily, not injected at session creation.** Resolution
  at execution time means a session created before the user connected still
  works, tokens are always fresh, and nothing long-lived is stapled to session
  state. The resolver is `UserConnectionResolver`, reached through
  `ToolContext`; see [Runtime](../foundations/runtime.md).
- **GitHub uses a GitHub App, not an OAuth App.** This is the central security
  decision of this concept. An OAuth App token lives forever and carries `repo`,
  meaning read, write, and admin over every repository the user can see. A
  GitHub App installation grants only the repositories the user selects, with
  granular permissions (`contents: read` for clone-only), and the server stores
  only the `installation_id` — not a secret — minting a one-hour installation
  token on demand from the App's private key. Blast radius drops from "all
  repositories, forever" to "selected repositories, one hour", and uninstalling
  stops new tokens immediately.
- **The credential never becomes text the model can read.** Tokens are absent
  from API responses, tool arguments, tool results, and message history, and git
  tools supply them through a credential-helper script inside the sandbox rather
  than a URL or command line, keeping them out of process listings and exec
  output. This is why credentials must be entered in Settings and not in chat:
  a secret typed into a conversation is persisted in the event log (TM-AGENT-016
  in the [threat model](../security/threat-model.md)).
- **Secrets resolve by explicit precedence.** A session secret wins over a user
  connection, which wins over an error carrying connect-here guidance. Session
  secrets are the narrower, shorter-lived grant, so they take precedence; the
  fallback is an error rather than an operator-wide credential.
- **Short-lived OAuth grants refresh without user involvement, and fail
  closed.** For MCP OAuth the resolver returns an unexpired access token or
  exchanges the stored refresh token, with a 60-second skew so it does not race
  expiry, coalescing concurrent reads of one grant and committing rotated token
  pairs atomically. Session-scoped MCP grants take precedence over the
  persistent connection. A rejected refresh returns reconnection guidance rather
  than a stale token. See [MCP Servers](mcp-servers.md).
- **One generic table, provider-specific code.** `user_connections` is
  provider-generic so GitLab and Bitbucket need no schema work, with two
  connection shapes: `oauth` (redirect to the provider) and `api_key` (entered
  in a Settings dialog). API-key providers self-register through
  `ConnectorPlugin` (`inventory::submit!`), contributing their display metadata,
  form schema, and an async `validate()` that runs before anything is stored, so
  adding a provider is a plugin, not a server change. See
  [Providers](../foundations/providers.md).
- **Uniqueness per (user, provider) is enforced in application code, not by a
  database constraint.** Multiple accounts per provider is a plausible future,
  and leaving the constraint out keeps that a code change rather than a
  migration.
- **The resolver stays backward compatible.** It checks `installation_id` first
  and falls back to a stored encrypted access token, so legacy OAuth connections
  keep working through the migration to Apps.

## GitHub App Configuration

The App itself is created in GitHub's console, so this is the one part of the
contract with no source file to point at.

| Variable | Meaning |
|---|---|
| `GITHUB_APP_ID` | Numeric App ID from the App settings page |
| `GITHUB_APP_PRIVATE_KEY` | PEM RSA private key used to sign the JWT; literal `\n` is accepted and converted at startup |
| `GITHUB_APP_SLUG` | Slug used to build the installation URL (default `everruns`) |
| `GITHUB_APP_SETUP_URL` | Post-install redirect (default `{AUTH_BASE_URL}/v1/user/connections/github/callback`) |

When creating the App: leave **Callback URL** blank — Everruns uses the
installation flow, not OAuth user authorization — and set **Setup URL** to
`https://<domain>/api/v1/user/connections/github/callback` with *Redirect on
update* checked. Webhooks stay off. Repository permissions are `Contents:
Read & write` (or read-only for clone-only deployments); no issues, pull
requests, or admin unless a capability needs them. `Everruns (Dev)`
(<https://github.com/settings/apps/everruns-dev>) is the pre-configured local
equivalent pointing at `http://localhost:9300`.

## Where the Details Live

| Detail | Source of truth |
|---|---|
| Endpoints, request/response shapes, and error codes | `crates/server/src/api/user_connections.rs` and the OpenAPI export |
| Connection row fields | `UserConnectionRow` in `crates/server/src/storage/models.rs`, see [Models](../foundations/models.md) |
| Connector plugin trait and registration | `crates/platform/src/connector.rs` |
| Lazy resolution and caching | `crates/server/src/storage/connection_resolver.rs` |
| Credential encryption at rest | [Encryption](../security/encryption.md) |
| Identity-owned connections | [Agent Identities](../runtime-resources/agent-identities.md) |

## See also

- [Integrations](integrations.md), the integration index and parity requirements
- [Runtime](../foundations/runtime.md), how tools reach the resolver
- [Threat Model](../security/threat-model.md), provider-credential threats
