---
type: Decision
title: "Agent Exposure"
description: "Agent-owned channels and triggers are the management and ingress boundaries."
tags:
  - everruns
  - integrations
  - agents
---
# Agent Exposure

Agents own their harness and expose communication through Channels. Triggers own
unattended execution. Each channel has independent authentication, runtime identity,
ownership and lifecycle; changing one channel must not change a sibling's authority.

Ingress resolves the channel and its agent, then checks both lifecycles before
checking credentials or finding a conversation. Sessions carry structural channel
or trigger attribution. Organization, owner and channel checks are mandatory for
session reuse and Slack actions.

The retired App aggregate is removed, including archival reads, App-scoped ingress
routes, response counts, evaluation targets, aliases, repository code and foreign
keys. [Applications](applications.md) are a separate publication and audience aggregate. Existing channel IDs and hosted addresses remain stable.

## Upgrade boundary

The removal migration preserves channels, conversation bindings, execution owners,
harnesses, virtual users and encrypted configuration. It completes unambiguous
session attribution before removing archival columns. It also converts scheduled
activity messages and routing tags. Unsupported historical eval targets become
external labels while retaining their original target as opaque history.

This is a contract migration: drain in-flight turns and deliveries, stop the old
API and workers, apply the migration, then start the new release together. Old
processes cannot run against the schema after these columns are dropped. Back up
the database first; rollback requires restoring the backup and the matching binary.

Merged migrations and immutable events/audit records remain unchanged. Historical
reserved tag prefixes remain blocked from user writes; removal must not turn
previous internal attribution into a user-controlled routing input. Historical
Slack provenance cannot authorize a new post; the next verified inbound message
establishes current channel provenance.

## Sources

- [Channels](channels.md), shared runtime and conversation bindings
- [Channel authentication](channel-auth.md), caller verification
- [Channel invocation](channel-invocation.md), webhook and scheduled execution
- [Agent triggers](../runtime-resources/agent-triggers.md), unattended work
- [Removal migration](../../crates/server/migrations/20261125151326_remove_retired_apps.sql)
