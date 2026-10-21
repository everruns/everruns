---
type: Spec
title: Continuous GitHub Agent sources
description: GitHub owns a connected Agent's portable package while the control plane continuously reconciles reviewed source into its existing identity.
tags: [agents, github, portability, synchronization]
---

# Continuous GitHub Agent sources

An Agent can bind its authored definition to a GitHub branch and package path.
This is an inbound deployment of the existing [portable package](agent-packages.md),
not a second definition format or an agent task. The control plane reads GitHub;
repository code is never executed and credentials never enter a package.

## Ownership

Connecting applies an explicitly reviewed commit and makes the Agent's authored
configuration read-only across every command transport. Binding changes and ordinary
definition writes lock the same Agent row. Only a task-local scope for the exact
organization and Agent permits reconciliation writes; no serialized request can
claim that authority. Package-owned channel configuration follows source ownership.
Lifecycle cleanup takes the Agent lock before Slack install locks; unnamed local
channels do not hold the Agent row while waiting for credential edits.
Publication, credentials and omitted destination channel bindings remain local.

The source binding stays outside package export. It captures the Agent's GitHub
connection, installation and stable repository identity. A removed or replaced connection fails
closed rather than silently substituting another identity. Repository access is
scoped to contents read. For a human-configured source, reconciliation rechecks
live organization membership and Agent management permission, then applies the
normal dependency, capability, feature and publication policies. An unattended
reconciler does not inherit an internal caller's permission bypass.

## Reconciliation

The existing durable cluster job machinery drains due sources, with bounded
concurrency and recoverable database leases. Periodic checks recover missed
webhooks and support deployments whose webhook URL is not public. Verified pushes
wake matching repository/branch bindings only within the receiving App's identities;
replays coalesce and event payloads never dictate an applied commit.

Files come from one immutable Git tree. The shared package codec selects assets,
validates paths and enforces package limits. Transport bounds accommodate GitHub's
base64 envelope while decoded package limits remain unchanged.
Symlinks, submodules, truncated trees
and missing dependencies fail explicitly. Git entry types distinguish package
folders from manifests, including folders whose names end in a manifest extension.
Stable Agent identity survives a package
name change; stable repository identity prevents a replacement repository taking
over an existing source.

Application, source revision and import history commit together. Binding generations
and lease ownership fence stale work after a reconnect, disconnect or recovery.
Errors preserve the last working definition and applied revision. Disconnect keeps
that definition and restores local editing. Agent Files keep the existing
[session seeding lifecycle](agent-packages.md#file-lifecycle).

## Boundaries and success bar

Two-way writes are deferred: they need conditional repository writes, a durable
outbox, shared base revisions, conflicts and a package editing contract that
preserves repository authoring structure. Export remains available for connected
Agents, and a copy is locally editable.

The [domain and reconciler](../../crates/server/src/domains/agents/source/mod.rs),
[GitHub materializer](../../crates/server/src/domains/agents/source/github.rs),
[lease repository](../../crates/server/src/storage/repositories/agent_sources.rs)
and [tests](../../crates/server/src/domains/agents/source/tests.rs) own exact shapes
and behavior. Acceptance includes full package application, transport-independent
edit rejection, failed-revision retention, atomic rollback, disconnect fencing,
connection loss and tenant-scoped push routing.
