---
type: Specification
title: "Channel Invocation"
description: "Channel-owned webhook and scheduled invocation, session reuse and execution authority."
tags:
  - everruns
  - integrations
---
# Channel Invocation

Webhook ingress uses the channel's canonical address and verifies channel liveness,
authentication and request deduplication before dispatch. New automation uses
[Agent triggers](../runtime-resources/agent-triggers.md). Existing schedule channels
retain their agent-owned run-now action and durable schedule binding.

Shared-session reuse is scoped by organization, structural channel or trigger ID,
owner and server-owned tags. Ephemeral invocation creates a new session. Execution
uses the channel's owner and virtual user; migrated triggers retain their stored
harness, owner and virtual user independently of later agent changes.

Templates expose agent, channel and invocation context. No App identity is resolved.
See [Agent exposure](agent-exposure.md) for the removal and upgrade boundary and
[the implementation](../../crates/server/src/domains/agent_channels/invocation.rs)
for the supported configuration and metadata.
