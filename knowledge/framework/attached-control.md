---
type: Specification
title: "Attached Control for Framework Hosts"
description: "Why the Framework ships a local control socket so a second process can list, watch, drive and cancel a host's running sessions, and the authorization, token and cancel rules it keeps."
tags:
  - everruns
  - framework
  - security
---

# Attached Control for Framework Hosts

Status: Implemented (2026-11-09, EVE-1340). Public guide:
`docs/framework/attach-control.md`; code:
[`crates/everruns/src/control`](../../crates/everruns/src/control.rs); example:
[`examples/attached-cli`](../../examples/attached-cli).

## Problem

Terminal hosts (yolop, `ercode`) want a second terminal to act on a live
session: list sessions, watch output, send input, cancel a turn. The Engine's
sessions live in one process, and the primitives (`Session::send`,
`TurnHandle::cancel`, `Session::events_from`) are in-process. yolop solved this
with its own 1,200-line control plane; every other host would have to repeat
it, including the part that is easy to get wrong: who may connect.

`Engine::attach` and `Session::admission` are not this. The first re-binds a
persisted session to a rebuilt Agent after a restart; the second reconstructs
an accepted-message handle on a hosting profile. Neither crosses a process
boundary or authorizes anyone.

## Decisions

- **Local Unix socket, not HTTP.** The use case is the same user on the same
  machine. Remote or multi-user access already has authenticated surfaces
  (AG-UI handler, serve's `/v1` API); a local plane must not become a third.
  Unix only; the feature does not compile elsewhere.
- **Two gates, both required.** Kernel peer credentials pin the OS user; a
  bearer token in a private file adds rotation and revocation, so a token that
  leaked into a log stops working without restarting the host. Both ends check
  directory ownership and mode, because a client in a shared temp directory
  could otherwise hand its token and input to a socket another user planted
  (TM-AUTH-040).
- **Registry, not engine-wide exposure.** Only sessions the host registers are
  reachable. The registry also holds each session's active `TurnHandle`,
  because the handle from `send` is the only cancel capability the facade
  exposes; host-originated sends must go through the registry or `track`.
- **Disconnect never cancels.** Attach streams, waiting sends, revocation and
  shutdown only end the control connection. Cancel is an explicit command
  with `TurnHandle::cancel` semantics.
- **Projected events.** Attach forwards text, tool names and turn boundaries,
  not tool arguments or results; the canonical envelope stays in-process.
  Durable sequences ride along so a client recovers from lag by re-attaching.
- **One request per connection.** Stateless dispatch keeps authorization
  per-connection and makes rotation simple: the client re-reads the token file
  once when refused.

## Success bar

A new host copies `examples/attached-cli` and gets list, attach, send and
cancel without reading yolop's control plane; the public guide states the
token lifecycle and cancel semantics. yolop can replace its registry, wire
types and socket handling with `everruns::control` and keep only its CLI
parsing.
