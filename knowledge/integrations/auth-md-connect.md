---
type: Decision
title: "auth.md connect for user MCP servers"
description: "Connect a person's MCP server through the auth.md agent-registration profile: the server registers, the person approves on the service, the identity assertion becomes the connection, and MCP tokens are minted from it."
tags:
  - everruns
  - integrations
  - mcp
  - identity
---

# auth.md connect for user MCP servers

> Status: **Proposed 2026-10-07**, not built. Extends
> [User MCP servers](user-mcp-servers.md); that page stays authoritative for
> ownership, the `user_mcp` capability and the Connect card.

## Summary

[auth.md](https://workos.com/auth-md) (WorkOS, open profile) lets a service tell
agents how to register themselves: the agent names the person's email, the
person approves the request inside the service, the agent polls and receives a
long-lived **identity assertion**, and exchanges it for short access tokens.
Shipmail's [auth.md](https://shipmail.to/auth.md) is the first real example we
tested against.

Everruns supports this as a third way to connect a user MCP server, next to
browser OAuth and literal headers. The server runs the protocol. The model only
asks for the connection and relays a link and a code.

## Why not let the agent follow auth.md itself

Checked on app.everruns.com with Platform Chat and Shipmail on 2026-10-07:

| Need | Hosted Everruns today |
|---|---|
| Reach the service at all | Refused: the system egress allowlist (`crates/contracts/src/runtime/system_allowlist.toml`) covers registries, code hosts, AI providers and clouds only |
| POST JSON to register, POST form to poll | `web_fetch` is GET/HEAD only; Platform Chat's shell has `enable_http` off |
| Keep a 30-day assertion secret and across conversations | Session secrets end with the session, and the model would see the value |
| Mint hourly tokens, one per resource | Nothing does it; Shipmail rejects a REST token on its MCP endpoint and the reverse |

Giving the model a general POST tool with credential injection and opening the
allowlist would make this work, and would also make every prompt injection an
exfiltration path. Moving the protocol into the connect flow keeps the model out
of credential handling and reuses the grant storage and refresh path user MCP
servers already have. It does not remove the allowlist question: see D7.

## Decisions

### D1. auth.md is a connect method, chosen from discovery

When a user MCP server's protected-resource metadata points at an authorization
server whose metadata has an `agent_auth` block with `service_auth` in
`identity_types_supported`, Connect offers **Approve in <service>** alongside
browser OAuth (when the server also supports it). Anonymous and ID-JAG
identity types are out of scope.

### D2. The person's email is the login hint, and they confirm it

`login_hint` defaults to the virtual user's verified email and is editable on
the Connect card. The card shows the service's `resource_name` and logo before
registering, as the profile asks. `agent_name` is the agent's display name,
capped at 64 characters.

### D3. The server polls; the chat shows link and code

Registration returns `verification_uri`, `user_code` and `interval`. The Connect
card shows the link and code and a pending state. A server task polls the token
endpoint at `interval` (adding five seconds on `slow_down`), restarts an expired
code through the claim endpoint while the 24-hour window is open, and stops on
`access_denied` or `claim_expired`. The poll survives the turn and a server
restart; the card updates when it lands. The claim token is never persisted
beyond the poll state and never reaches the model.

### D4. The identity assertion is the grant

On success the assertion is stored as the virtual user's ordinary
`mcp_oauth_<server uuid>` connection, encrypted, with its expiry. It shows up,
expires and revokes like every other connection. The access token returned by
the claim is cached as the first access token.

### D5. Tokens are minted per resource through the existing refresh path

The connection resolver's refresh step learns one more exchange: for an auth.md
grant it posts the RFC 7523 jwt-bearer grant with `resource=<the MCP server
URL>` instead of a refresh token. That is how "the REST API and the MCP server
each get their own token" is honored: the MCP client only ever asks for the MCP
resource. `invalid_grant` (the person disconnected the agent in the service, or
the 30 days ran out) marks the connection broken, and the next call shows a
Connect card again.

### D6. The protocol code lives in everruns-core

Discovery, register, poll, restart and exchange go in
`everruns_core::mcp::oauth` next to the browser flow and return the same
`TokenSet`. The server adds persistence and the poll task; yolop gets the same
flow with its own token store and a terminal prompt in place of the card.

### D7. Services stay behind the curated allowlist

In hosted production the server and worker run with
`EVERRUNS_SYSTEM_ALLOWLIST_ENABLED=true`, and MCP traffic, server-side OAuth
discovery and token calls all go through the same runtime egress boundary
(`DirectEgressService::for_runtime_traffic_from_env`). The allowlist exists so
open-signup users cannot relay through arbitrary hosts, so a person adding a
server URL must not widen it on its own.

A service is usable on the hosted app once its hosts are in a reviewed
`connected_services` group of `system_allowlist.toml` (Shipmail first:
`shipmail.to`). Self-hosted deployments without the allowlist need nothing.
Rejected: a per-connection exception for whatever URL the person enters, because
that reopens the relay the allowlist closes.

## Out of scope

- A general HTTP tool that can POST with injected credentials.
- Per-connection or per-org egress exceptions.
- REST-only services without an MCP server. They need the HTTP tool above.

## Plan

1. Core: `agent_auth` metadata parsing and the register/poll/restart/exchange
   client, with wire tests against a stub server.
2. Server: Connect method, poll task, grant storage, jwt-bearer refresh, card
   states (pending, approved, denied, expired).
3. Allowlist: add the `connected_services` group with `shipmail.to`.
4. Platform Chat: no change beyond `user_mcp` manage; verify end to end with
   Shipmail (`https://shipmail.to/api/mcp`).
5. yolop: `/mcp login` uses the same flow when the server advertises it.
