---
type: Decision
title: "MCP sign-in health: no silent loss of an agent's MCP server"
description: "An agent's OAuth MCP sign-in that can no longer produce a token is marked, reported in Health ahead of and at expiry, shown with its real state and Sign in again on the agent's MCP sheet, available as `everruns agents mcp list`, and replaced in turns by a sign_in_required tool."
tags:
  - everruns
  - integrations
  - mcp
  - health
---

# MCP sign-in health

> Status: **Accepted and built 2026-11-22.** Extends
> [Agent MCP attachments](agent-mcp-attachments.md) and
> [User MCP servers](user-mcp-servers.md).

## Problem

An agent acting as its own service account (`actsAs: service` or
`user_or_service`) signs in to an OAuth MCP server once. When that grant could
no longer produce a token, resolution returned nothing, the server's tools
dropped out of every turn, and nothing else changed: the MCP tab still said
"Connected", Health had no issue, and the model had no hint. The Kids Duties
agent lost its stend and visti servers this way for 11 days (2026-11-10 to
11-21) and simply stopped updating its page.

The trigger there was a refresh token saved before grants were bound to their
server ([TM-MCP-008](../security/threat-model.md)): such a token is never sent,
so the grant dies when its access token expires (stend issues 30-day tokens).
The same silence followed a provider refusing a refresh (`invalid_grant`), which
deleted the grant, and a server rejecting the token (401), which deleted it too.

## Decisions

1. **Mark, never delete.** A grant that cannot produce a token gets
   `provider_metadata.reconnect_required = {reason, at}`
   (`crates/server/src/storage/connection_resolver/grant_health.rs`). Reasons:
   `unbound` (saved before binding), `settings_changed` (bound to settings that
   changed), `no_refresh_token`, `refused` (`invalid_grant`), `rejected` (the
   server kept rejecting it). Resolution returns nothing for a marked grant
   without asking the provider again. Signing in again replaces the row, which
   clears the mark; a successful refresh clears it too. People's records are
   never purged.
2. **Unbound grants are still not refreshed.** Guessing the audience from
   settings that may have changed is the substitution TM-MCP-008 closes. The
   access token works until it expires; the warning below gives people time
   to sign in again first.
3. **A first 401 forces one refresh.** When the grant can renew itself, a
   server rejecting its token expires it (next use refreshes) and notes
   `rejected_at`; a second rejection within 10 minutes marks it `rejected`.
   A transient refresh failure (network, 5xx) marks nothing.
4. **Health issue `mcp.sign_in_required`**, one per agent and server
   (`crates/server/src/domains/health_issues/mcp_grants.rs`). It opens
   3 days before an unrenewable grant expires ("expires soon") and stays open
   once it needs sign-in; the step to "needs sign-in" tells people again. The
   agent's authors (or the org's owners) are notified. The issue links the
   agent's MCP tab, since only a person can sign in. It resolves once the grant
   is usable again or the server leaves the catalog. The 15-minute sweep only
   reads grants: a v2 org's grant is refreshed on its data plane, and a refresh
   from the control plane would spend the refresh token the data plane holds.
5. **The MCP sheet shows the real sign-in state.** Users read "Connected as
   stend" for 11 days while the tools were gone, so each attachment carries
   `sign_in` (`working`, `expires_soon` with `expires_at`, `needs_sign_in`
   with `since`, plus the reason), read from the stored grant without
   contacting the server
   (`crates/server/src/domains/agents/mcp_attachments.rs`). A grant that cannot
   renew is shown as "stops <date>" as soon as it is seen (Health waits for
   the last 3 days), with "Sign in again" for managers. A stopped grant makes
   the attachment `reconnect_required` and can still be revoked. The sheet is
   named "MCP".
6. **The model is told.** A server that signs in as the agent and has no usable
   login lists one tool, `mcp_<prefix>__sign_in_required`, in place of its
   tools: its description says the tools are unavailable and a person must
   sign in again on the agent's MCP tab, and a call takes the ordinary MCP path
   (no credential, so `connection_required` with the sign-in step). On the
   control plane it replaces the tools when no credential resolves; in the
   runtime (data plane cells, embedded hosts) when the server rejects the
   credential with 401/403. A person's own login (`actsAs: user`) is theirs to
   add and keeps its old behavior. The placeholder is never stored as the
   server's last listed tools.
7. **Same states in the CLI, MCP and Platform Chat.** Listing and revoking
   agent MCP sign-ins are commands (`everruns agents mcp list`,
   `everruns agents mcp revoke`), not console-only routes, so Platform Chat
   can name the broken server, its reason and the sign-in link. Signing in
   stays a person's step in the browser.
8. **Marked grants do not go to a data plane.** `agent_service_mcp_grant`
   skips them, so a cell sees no grant, its server rejects the call, and the
   runtime lists the placeholder.

## Known limits

- A data plane cell whose refresh is refused does not report it back; the
  control plane sees the grant expire with no refresh and reports it only when
  it cannot renew it (unbound, no refresh token, or marked by a v1 turn).
- A person's own grants (`actsAs: user`) get no Health issue.
