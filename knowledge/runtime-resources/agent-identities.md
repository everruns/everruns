---
type: Specification
title: "Agent Identities"
description: "Agent identities (virtual principals for unattended execution)."
tags:
  - everruns
  - runtime-resources
---
# Agent Identities

## Abstract

Agent identities are durable virtual principals. They are separate from `Agent` behavior/configuration and can be attached to `Session` and `App` resources to represent who an unattended or channel-driven execution is acting as.

See `crates/core/src/agent_identity.rs` for the full field list.

## Concepts

### Separation of concerns

- `Agent` = behavior, reasoning, tools, capabilities.
- `AgentIdentity` = virtual principal: name, avatar, locale/timezone defaults, lifecycle.
- `Session.agent_identity_id` = resident identity for unattended/background execution defaults.
- `App.agent_identity_id` = resident identity for channel/app execution defaults.

This keeps interactive chat semantics intact: the human user still initiates and acts in live UI turns unless a future workflow overrides that explicitly.

### Execution provenance

New work that injects messages autonomously should record both:
- `initiator`
- `acting_principal`
- `initiator_principal_id`
- `acting_principal_id`

These values belong in event metadata, not in message content. `external_actor` remains a separate concept for channel speakers.

Agent identities also participate in durable ownership through the principal graph:
- each `AgentIdentity` has a corresponding `agent_identity` principal
- that principal keeps its existing parent/effective user when the identity is edited
- sessions and apps that reference the identity can use the identity principal as their durable owner without losing the effective human owner

### Identity per agent (lazy default)

Agent identities can be created and managed standalone (the CRUD API above), but
an agent also gets one **lazily, on demand**: the first time an agent acts
unattended (currently an agent-trigger fire, see `knowledge/runtime-resources/agent-triggers.md`) it
is given its own `agent_identity` so it owns that work as itself.

- The link lives on `agents.agent_identity_id` (storage-only; not on the public
  `Agent` API). It is nullable and created on first unattended action.
- Creation is idempotent and never overrides an existing link: an identity
  attached explicitly, or from a prior fire, is reused. The guarded link write
  only sets `agent_identity_id` while it is still NULL, so a concurrent
  first-fire race resolves to a single winner (the loser's orphan identity is
  soft-archived best-effort).
- The lazily-created identity's principal is parented to the org system-owner,
  so the effective human/system owner and budgeting are unchanged; the agent
  simply acts as itself.
- No retroactive rewrite: existing agents and their historical sessions are left
  as-is and pick up an identity on their next unattended action.

### Scheduling

`SessionSchedule` does not own a separate identity field. Schedules inherit the resident identity from their session at execution time.

## Lifecycle

Agent identities follow the standard building-block lifecycle:

`active -> archived -> deleted`

Contract:
- archived identities stay readable for historical references but cannot be newly assigned to sessions/apps.
- deleted identities are tombstones and should not be returned from normal detail/list flows.

## UI / API scope in this change

This implementation introduces:
- CRUD API for agent identities.
- Frontend management pages.
- Session/App assignment UI.
- Event provenance metadata for interactive, scheduled, and app-driven input messages.
- Identity-owned connections: when a session has `agent_identity_id`, connection resolution prefers `agent_identity_connections` over `user_connections`, falling back to user connections only if no identity connection exists for the requested provider.

It does **not** yet introduce identity-owned inboxes. That remains follow-up work on top of the principal model.
