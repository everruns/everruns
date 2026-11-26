---
type: Specification
title: "Injected Capabilities"
description: "Hosted capabilities a controller serves over MCP to whichever data plane runs the session: the definition, session tokens, the Everruns MCP extensions, and what is built so far."
tags:
  - everruns
  - execution
  - capabilities
  - mcp
  - data-plane
---

# Injected Capabilities

## Abstract

A data plane (a v1 worker, a v2 shard, standalone agentd) runs only **native**
capabilities: files, shell, web, subagents, tasks, schedules, session state,
SQL databases. Capabilities that need the control plane's data (Knowledge,
Memory, Slack actions, Poppy, platform commands, the MCP store, connection
lookups) are **injected**: the control plane serves them over MCP, and the data
plane calls them like any MCP server, with a session token it signed.

This replaces porting each hosted capability into the shard and replaces the
host-call path. One implementation stays on the server; the data plane needs no
code per hosted feature, and agentd can run without the server at all.

## Requirements

### Definition

An injected capability is a declarative capability
([Capabilities Specification](capabilities.md)) plus one MCP server the
controller runs. Its id is `injected:{name}`. Fields beyond the declarative
shape:

- `server.url`: the Streamable HTTP endpoint. It is also the token audience.
- `attach`: `on_request` (default, the agent lists `injected:{name}`) or
  `always` (every session whose labels match).
- `match`: session labels that must all match. Empty matches every session.
- `tools`: `prefixed` (default, `mcp_{name}_{tool}`) or `native` (the server's
  own names, so converting a hosted capability does not rename tools agents
  already call).

Hosts collect it as a declarative capability whose MCP server has
`auth_mode: session_token`. Source of truth:
`crates/contracts/src/runtime/capabilities/injected.rs`.

User-authored MCP servers (declarative, plugin, scoped attachments) may not use
`session_token`; validation refuses them. Only a controller registers injected
capabilities.

### Session tokens

The data plane signs one EdDSA (Ed25519) JWT, the key type of its org tokens, per call (`typ: everruns-session+jwt`,
5-minute TTL): `sub` session, `grp` the group label value (the organization in
an Everruns deployment), `cap` the capability id, `aud` the exact server URL,
and `act` (RFC 8693, `{"sub": <public user id>, "turn": <input message id>}`)
the person whose message opened the session's turn, and that message, when
one did.
The server verifies signature, `typ`, issuer, audience, `cap` and expiry against
the JWKS of the data planes it trusts. A token for one server never verifies at
another. Source: `crates/core/src/mcp/session_token.rs` (feature
`session-token`).

THREAT: a session token is a bearer credential for one session in one
organization. It is minted per call and never stored, and the audience pin
stops a server from replaying it elsewhere. The server resolves every lookup
inside `grp`, so a session id from another organization reads as missing. The
data plane vouches for `act` from its own record of who sent the turn, never
from the turn's tools; a server that acts as the person rechecks that they are
a member of `grp`, with their current role, and refuses without one.

A data plane of an Everruns deployment signs with a key derived from its link
key (`dataplane_seal::session_token_pkcs8`), so every executor signs with the
same key and the control plane trusts its linked data plane without a JWKS or a
key to distribute. `INJECTED_CAPABILITIES_JWKS` adds other data planes.

### Everruns MCP extensions

The host declares `io.everruns/facts`, `io.everruns/files`,
`io.everruns/hooks` and `io.everruns/ui` under `capabilities.extensions`, only
to session-token servers. A server lists the ones it speaks in the
`everruns://extensions` resource (MCP's stateless era has no handshake to carry
server capabilities):

| Extension | What it does |
|---|---|
| instructions | the capability's system prompt (declarative `system_prompt`, also `initialize.instructions`) |
| facts | `everruns://facts` resource read at turn start, capped at 32 facts of 4 KiB |
| hooks | `everruns/hooks/beforeTool` (allow, rewrite arguments, deny) and `everruns/hooks/afterTool` (replace result or error, add context) for the tools the server subscribed to |
| files | a file tree the host mounts: one method per operation (`everruns/files/read`, `.../write`, ...); `read` and `stat` return an `etag` (content hash) and `write` takes `if_match`; paths are absolute and normal, and the server resolves them inside `grp` and the session's own tree |
| ui | `structuredContent` and `_meta` on tool results, rendered by the host |

A `beforeTool` hook that cannot be reached blocks the call; an `afterTool` hook
that cannot be reached leaves the result unchanged. The serving handler refuses
a file path with `.`, `..` or empty segments before a service sees it, and
turns `if_match` into a compare-and-set on the content the tag names, so a
service implements only the plain operations. Wire types:
`crates/contracts/src/runtime/mcp_extensions.rs`; host side:
`crates/core/src/mcp/extensions.rs`; serving side:
`crates/core/src/mcp/injected_server.rs`.

### Control plane endpoint

`POST {api}/v1/injected/{name}/mcp`, outside the user-auth stack. It is off (404)
until `INJECTED_CAPABILITIES_JWKS` and `INJECTED_CAPABILITIES_ISSUERS` name the
trusted data planes. Every authentication failure is the same 401. Source:
`crates/server/src/injected/`.

The URLs it hands data planes (and the audiences it checks) use
`INJECTED_CAPABILITIES_BASE_URL` when set, the address data planes reach it on
(a cluster service, `127.0.0.1` locally), including the versioned API prefix
(`/api/v1` by default). An API root such as `/api` gains `/v1`; an explicitly
versioned base keeps its version. Discovery and session-token audiences use
the same normalized endpoint. A shard
trusts its configured control plane's origin (`AGENTD_CONTROL_PLANE`) and skips
the MCP client's private-address check only there.

THREAT: the exemption follows operator configuration, never a URL an agent,
tool or package names; every other MCP server keeps the DNS-pinned check.

## Status

Built (side by side with today's paths, nothing switched over):

- Contract types, session tokens, the host-side extensions client and hook
  adapters, the transport-free serving handler.
- Knowledge Base served at `/injected/knowledge_base/mcp`, reusing the existing
  `search_knowledge` tool; the session's config comes from its agent's
  `injected:knowledge_base` or `knowledge_base` entry.

- Shards mint session tokens per call (`everruns-agentd` `mcp.rs`), with the
  key derived from their link key; the control plane trusts its linked data
  plane from that key alone.
- Hosts collect `injected:` refs whose config is the controller's
  definition (`carries_declarative_definition`); a bare one contributes
  nothing.
- When a data plane is trusted, the control plane swaps the hosted
  capabilities it serves for their injected refs in every package it hands
  a v2 data plane (`server/src/injected/mod.rs` `attach`), in place, and
  keeps a hosted capability's native part beside it (Slack's model view as
  `slack_model_view`).
- Served: Knowledge Base, Poppy client and Slack (`hosted.rs`: the
  capability's own tools, run in process by the call dispatcher that runs
  each call's v1 worker command, `server/src/domains/data_plane/calls.rs`). Slack's `slack_upload_file` reads the session's file
  from its shard through the `fs` session command (`io.everruns/files` serves
  the control plane's trees to data planes, not the reverse). Platform stays native (its tools, `/docs` mount, skills and the shell's
  `everruns` builtin); only its command catalog is served, as the
  `platform_commands` server whose one tool runs a command as the token's
  `act` (`hosted.rs` `platform`; `everruns-agentd` `platform_calls.rs`).
  The same server answers the session-scoped operations a native tool
  needs from the control plane: the session's spend (`get_session_info`,
  as no one) and its agent's saved scripts (`tools scripts`: a list as no
  one, a save as the turn's sender; `everruns-agentd` `saved_scripts.rs`).
- Memory is a remote mount: the control plane serves it as the
  `memory_files` files server (`hosted.rs` `memory`, with the v1 worker's
  mount check). The URLs of these control plane servers
  (`CONTROLLER_SERVERS`) come with each create, never from a package; a shard
  mounts the session's Memory from it with a session token (`everruns-agentd`
  `memory.rs`); a create with no files server mounts no Memory.
- The turn's person's own MCP servers (`user_mcp`) stay native in the shard
  (the manage tools, the per-turn servers hook); their list and its
  resolution are the `person_mcp` server's (`hosted.rs` `person_mcp`): one
  tool for the manage tools' store and one for the servers a turn adds. The
  token's `act` names the person and the open turn's input message
  (`SessionActor.turn`, from the same session record), so the tools take
  neither as an argument, and the control plane records the turn's
  invocation for that person first (`everruns-agentd` `user_mcp.rs`).
- Connection lookups are one credential lookup: the `credentials` server
  (`hosted.rs` `credentials`) has one tool for the four credential calls
  (`CREDENTIAL_CALLS`: provider connections, MCP sign-ins, GitHub, service
  keys), each running its v1 worker command. A shard mints its token
  acting as the sender of the message the lookup names (`"open"`: the open
  turn's; none: no one), from the session's record, so a detached tool
  call of an earlier turn still acts for that turn's person
  (`everruns-agentd` `lookups.rs`, `mcp.rs` `Acting`). Org connections and
  settings stay pushed config. A shard with no credentials server (no
  control plane) finds no connections.

Next, in order:

1. Agentd publishes JWKS and mints tokens.
2. Hosts honor `tools: native` and load facts and hooks per turn.
3. Step 0 on data plane v2: convert the hosted capabilities, then delete the
   host-call path (done 2026-11-22). The audit found none uses facts or tool hooks; Slack's
   model view and Platform's static `/docs` mount stay native, Memory becomes
   a remote mount, and connection lookups become one native credential lookup
   (pushed org connections plus a per-turn, person-scoped token fetch) served
   by the same token service. That lookup is the only call left that crosses
   planes.
