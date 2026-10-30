---
type: Specification
title: "Slack Code"
description: "Durable task channels, shared session routing, review projections and requester-bound hard approvals for Slack Code."
tags: [everruns, slack, integrations, coding]
---

# Slack Code

## Product contract

Slack Code is an explicit endpoint opt-in. Channel mentions start dedicated coding
workspaces; ordinary DMs keep their existing behavior. With the option off, normal
Slack routing is unchanged. An endpoint intended for general conversation should
keep the option off rather than create a code channel for every question.

Each task has one fresh Everruns session and its existing sandbox lifecycle.
Only the initiating message is admitted into a new task: no prior DM or thread
history is promoted. Teammates, channel messages, thread replies, and follow-ups
in the origin thread all steer that session with external-actor attribution.
The original requester remains the approver regardless of who steers next.

The existing Slack transport still owns signature verification, endpoint
liveness, installation, file intake and credentials. The Code worker owns durable
intake and task projection, using the shared session and message services. It does
not enable the transport-wide shared binding or grant Slack members dashboard or
credential access.

Sources: [worker](../../crates/server/src/channels/slack/code/worker.rs),
[ingress and durable authority](../../crates/server/src/channels/slack/code/store.rs).

## Installation and isolation

Manifest generation, OAuth consent and health checks include the Code management
and conversation-inspection permissions only for opted-in endpoints. Existing
apps must update their manifest and reinstall. The setting alone cannot grant a
Slack permission. The configured workspace must match the signed event envelope.

A Slack-created channel is adopted only after authenticated conversation info
identifies its immutable agent-channel record type, this installation's bot is
assigned, and the bot is a member. Restricted endpoints additionally require the
configured origin channel (or the explicitly configured code channel itself).
Unrelated channel events cannot bypass the source restriction. Installation-token
changes require verification before the old task can publish or act again.

Creation always supplies the origin and a stable, locally bound session identity.
It never supplies a privacy override: Slack inherits origin privacy, adds the
requester, and refuses unsupported Slack Connect origins. These are Slack's
external contracts, not permissions Everruns can grant itself.

## Durability

Signed Code inputs are persisted before the webhook is acknowledged. A message
and its mention event share an inbox identity; a mention upgrades an earlier plain
message even when the two arrive in reverse order. Work is serialized in the
database, not by an in-memory mutex.

The durable creation intent and local session binding precede external creation.
A retry reuses the same session identity, including after an ambiguous remote
success. Message admission uses the shared service's stable client-message key,
so replay cannot execute the initiating input again. The Code worker is retained
by the server supervisor and reconciles unfinished inputs after restarts.

Reply delivery uses a persisted event cursor and stable Slack client-message
identities. API timeouts may still leave ambiguous outcomes; reconciliation and
idempotent projections are the contract, not universal exactly-once external
message delivery. Network calls have bounded timeouts and rate-limit advice is
persisted as the next eligible attempt.

## Stops and lifecycle

Stops normalize root and legacy coordinates and reject conflicts. Channel-wide
and threaded Code stops resolve the same task. An independent durable stop marker
commits before acknowledgment; it does not wait behind the projection row's
network request. Cancellation is triggered immediately and retained in the inbox
for recovery. Session background tasks receive cooperative cancellation intents.

Every later outbound projection checks the marker. An already in-flight request
may finish, but no subsequent message, status or review-surface call starts from
that stopped projection. A fresh human input can resume the same task; inputs
queued before the stop are discarded, old output is skipped, and a replayed stop
cannot cancel newer work. Archive/reopen is a separate lifecycle operation.

For Code stops, follow the [Code lifecycle guide](https://docs.slack.dev/ai/slack-code/lifecycle):
stop work and further output; let Slack update its stop UI. This deliberately
selects the Code-specific contract over the conflicting generic event reference.
Agent-pane sessions retain their own cancellation-notice behavior.

Session lifecycle is separate from progress narration. Processing, parked hard
approvals, ready sessions, and explicit task closure remain distinct. See
[Channels](channels.md) and the [projection](../../crates/server/src/channels/slack/code/projection.rs).

## Review projection

Code sessions receive the native Slack capability and at least the Normal enforced tool
approval gate, preserving inherited Protective mode. The publication tool reads a bounded UTF-8 diff file from the
session filesystem; it does not accept model-authored inline diffs. Agents must
generate that file from the repository with a known base and record validation.

Publication replaces a complete desired snapshot, guarded by its expected
revision. Identical retries return the same receipt. Stale updates are refused
with the current revision rather than overwriting a newer snapshot. The receipt
means queued, not that Slack has already accepted every surface update.

The runtime publishes the complete context bar, a singleton diff view and a stable
Block Kit summary view. It reserves one context item for the authenticated
Everruns session link. Content/count limits are validated on both sides of the
worker seam. Oversized diffs must use a review link instead. Views are upserts;
transient failure retains the desired snapshot for reconciliation.

Source: [publication tool](../../crates/capabilities/src/capabilities/slack/code_task.rs).

## Enforced approvals

The Code projection renders only engine-authored hard approval requests as hard
approval cards. A signed click must match the endpoint, workspace, channel,
original requester, exact pending request and fingerprint. Expired requests,
changed actions, unauthorized clickers, and replays fail closed. The shared
[tool approval resolver](../execution/tool-approval.md) persists the decision and
resumes parked execution; Slack prose does not resolve the gate.

Decisions are attributable to the signed Slack actor. Credential-storage approval
retains its authenticated-UI requirement and cannot be granted through a Slack
card. Delegated approvers, HTML/canvas collaboration and native GitHub review
parity remain explicitly excluded from this vertical slice.

Source: [Code interactivity](../../crates/server/src/channels/slack/code/interaction.rs).

Signed hard-approval clicks enter the durable inbox before acknowledgment. Task
locks, gate resolution and card updates run in the worker, so projection I/O
cannot consume Slack's interaction response deadline. Retries share one input;
crash recovery still revalidates the current pending gate and expiry.

## Completion and invalidation

An explicit complete snapshot requires a final summary. Once the turn settles,
the worker publishes the summary, records its timestamp, closes the task and
reconciles archive. Repeating an already-completed archive is a satisfied outcome.
A verified unarchive reconnects to the same session, clears the old completion
snapshot and uses the runtime's existing sandbox restoration behavior.

Uninstall and token revocation durably suppress projection and queue cancellation.
Disabled endpoints and invalid credentials fail closed; an unavailable transport
cannot select a sibling endpoint or another workspace's token. A revoked task is
not silently transferred to a new installation. Existing task history remains in
Everruns under its original authorization policy.

## Qualification

[Database-backed acceptance tests](../../crates/server/src/channels/slack/code/tests.rs)
exercise real session/message persistence and HTTP contract fixtures for duplicate
and reversed events, ambiguous creation recovery, shared steering, source/ownership
isolation, stop/restart behavior, publication bounds/revisions/rate limits, hard
approval replay/expiry/changed arguments, invalidation, archive and reopen.
Ordinary Slack and pane regression tests remain required.

Live qualification is currently blocked: the configured test installation
successfully authenticates but Slack rejects Code methods for missing Code
management permission. Updating an app requires app-management authorization and
renewed workspace consent; a bot token cannot grant those to itself. This is an
external validation blocker, not a completed live acceptance result. Hosted SaaS
adoption remains a separate pinned-OSS rollout and must not replace its submodule
with the sibling checkout.
