---
type: Spec
title: Continuous GitHub Agent sources
description: A GitHub branch and an Agent share a portable package through continuous inbound or opt-in two-way synchronization.
tags: [agents, github, portability, synchronization]
---

# Continuous GitHub Agent sources

An Agent can bind its authored definition to a GitHub branch and package path.
The existing [portable package](agent-packages.md) is shared in both directions.
The control plane reads and optionally commits source; repository code is never
executed and credentials never enter a package.

## Ownership

Connecting applies an explicitly reviewed commit. Pull mode, the default for new
and existing bindings, makes authored configuration read-only across every command
transport. Opting into two-way mode requires repository-scoped Contents write
permission and permits normal authored edits through UI, API, CLI and MCP. The
[Chat mode draft adapter](../ui/chat-mode.md) follows the selected mode. Binding changes and ordinary
definition writes lock the same Agent row. Only a task-local scope for the exact
organization and Agent permits reconciliation writes; no serialized request can
claim that authority. Package-owned channel configuration follows source ownership.
Lifecycle cleanup takes the Agent lock before Slack install locks; unnamed local
channels do not hold the Agent row while waiting for credential edits.
Publication, credentials and omitted destination channel bindings remain local.

The source binding stays outside package export. Sync is an authoring connection,
separate from the Agent's runtime GitHub identity on its Integrations tab: changing or
removing that identity never touches the source. The person setting up sync chooses the
account it reads and writes through: their own GitHub sign-in (two-way commits carry their
name; the source pauses if they disconnect or leave the organization), or the organization's
link to the shared Everruns App for the repository's account (commits come from the App's bot
and survive people leaving; only admins link accounts). Both connect in a popup from the Sync
dialog, which lists the repositories each account reaches. Bindings made before this split
keep the Agent's identity until they are reconnected. The binding captures the chosen
connection, its installation when it has one, and stable repository identity. A removed or
replaced connection fails closed rather than silently substituting another identity. Repository access is
scoped to contents read for pull mode, and contents write only for explicitly
selected two-way mode. Existing GitHub Apps need the owner to grant and accept
write access; creating an App does not silently broaden its permissions. For a human-configured source, reconciliation rechecks
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
Errors preserve the current local definition and last applied revision. Disconnect
keeps the current definition and restores unrestricted local editing; it stops new
attempts, while a GitHub request already in flight may still complete. Agent Files keep the existing
[session seeding lifecycle](agent-packages.md#file-lifecycle).

## Boundaries and success bar

Two-way synchronization compares local authored changes and the selected package
against their shared baseline. An unrelated repository commit is retained as the
parent of an outbound change. Concurrent package changes pause with a visible
conflict rather than choosing a winner. Reviewing and reconnecting the GitHub
version explicitly replaces local edits; disconnect keeps local edits without
changing the repository.

Linked skill exports serialize metadata deterministically so unchanged package
bytes cannot create a write-back loop. Local edits queue reconciliation in their
definition transaction. Outbound commits
are persisted before publishing; uncertain responses recover the same commit,
including when newer commits already descend from it. Queued retries revalidate
current package policies before making a new branch update; acknowledgment of an
already-delivered commit does not replay that update. Non-forced branch updates
preserve concurrent repository commits and respect branch protection. Replaced or
disconnected generations cannot start publishing a queued write. Intermediate
local edits remain queued if a newer edit arrives while an earlier snapshot is
being published.

Write-back keeps the selected manifest path and format, external instruction path,
and existing runtime asset source paths, including remapped files and legacy skill
roots. Changed asset sets become explicit declarations; new assets can remain
inline without claiming unrelated repository files. Unchanged manifests retain
their bytes; manifest edits may normalize formatting and comments. Only authored
fields changed locally are replaced, preserving values normalized by import and
local publication decisions. Unnamed destination channels stay local. Workflows
cannot be changed by source sync. Export and copy remain available in both modes.

The [domain and reconciler](../../crates/server/src/domains/agents/source/mod.rs),
[GitHub materializer](../../crates/server/src/domains/agents/source/github.rs),
[lease repository](../../crates/server/src/storage/repositories/agent_sources.rs),
[outbound writer](../../crates/server/src/domains/agents/source/write.rs),
[shared package editor](../../crates/core/src/agent_package/edit.rs),
[inbound tests](../../crates/server/src/domains/agents/source/tests.rs) and
[write regressions](../../crates/server/src/domains/agents/source/write_tests.rs) own exact shapes
and behavior. Acceptance includes full package application, transport-independent
edit rejection, failed-revision retention, atomic rollback, disconnect fencing,
connection loss and tenant-scoped push routing.
