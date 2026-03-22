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

These values belong in event metadata, not in message content. `external_actor` remains a separate concept for channel speakers.

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
