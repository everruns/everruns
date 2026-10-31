---
type: Specification
title: Work Chat Channels
description: Published consumer work experiences with capability-qualified interfaces and isolated browser identity.
tags: [channels, chat, identity, workspace]
---

# Work Chat Channels

Work Chat is an Agent-owned channel. It publishes one configured experience,
with its own lifecycle and consumer access, and does not revive the retired App
aggregate. Several channels may expose the same Agent independently.

The publisher chooses a single main conversation with work threads, or multiple
independent conversations. Single-with-threads requires the effective Agent to
provide coordinator thread tools. The shared feature resolver checks inherited
capabilities, their configuration and effective rollout availability. Publication
and configuration mutations use that same contract; invalidating a live channel
rolls back the mutation. Runtime rechecks protect against deployment changes.

Consumers are virtual users, not console members. Channel-bound runtime tokens
reach consumer session and self-service routes only. Who may sign in is the
channel membership: open (anyone the deployment identity provider accepts),
invites (one verified email plus invited emails), or organization users
(a current member, matched by external id or by a verified email on a verified
account). An unverified email is not an identity. The deployment provider is
configured, not hardcoded. A publisher-chosen provider is a later membership
variant. Personal access tokens for organization members stay an execution
auth method on API channels.

Sessions, workspaces, tasks, questions and approvals use existing services.
The channel introduces no retention policy. One main conversation is bound per
channel and consumer; concurrent creation returns the same winner. Independent
conversations and work threads remain distinct sessions, with workspace and
budget-root semantics preserved.

Thread delegation has an explicitly bounded consumer path shared by local and
RPC workers. A verified invocation can use the configured workers and its own
conversation tree, without gaining management authority. Channel and consumer
ownership are carried onto the child sessions. General platform commands still
require the independently verified management invocation.

Work Chat projects relevant conversation and work events rather than debug or
provider reasoning records. Workspace operations use the authorized session's
workspace, respect read-only files, normalize aliases before validation, and
exclude private runtime mounts. Revision-checked saves reject stale edits. Shared legacy workspaces are not exposed to
consumers. Question and approval decisions retain their existing gates.

Existing Agent, channel and session budgets apply. The execution API's caller
spending limit is shared; it is an admission check, not a prepaid reservation.

## Browser host

The optional standalone browser host in `apps/chat-ui` serves one channel per
hostname. The deployment configures its domain and OIDC provider; no hosted
vendor or cloud domain is hardcoded upstream. Each Work Chat is assigned a
stable DNS label when the channel is created. That label is not the channel
id. The hostname, never a browser-supplied API URL, org, or channel id, selects
the channel: the host asks the API which channel owns the label.

Browser login uses a fixed broker callback with state, nonce and PKCE. Durable,
single-use handoffs bind the destination channel and initiating browser.
Host-only opaque cookies reference encrypted server-side sessions. Ciphertext
is authenticated against the credential class and cookie hash, and refresh-token rotation is locked
across replicas. Origin checks protect cookie-authenticated mutations, including
against sibling hosts. Login admission is rate limited. Stream reconnects renew
channel authority so changed auth and publication policy takes effect.

The host's database role needs only the browser credential/admission table.
It has no platform-management credentials. The backend-for-frontend allowlist
reaches consumer execution and self-service routes only.

## Browser recovery

The host supplies a random, non-credential recovery namespace after authenticating
the browser session. It changes on login and remains stable across credential
refresh. Tab-local drafts and unconfirmed message requests are bound to that
namespace and conversation, including the original Agent API idempotency key.
Reload never resends them automatically. Retry preserves the original body and
key; requests beyond the local retry window require review rather than silently
exceeding the server's idempotency retention. Discard removes only the local copy.
Logout clears all local recovery. Legacy sessions without a recovery namespace
keep memory-only behavior and show the storage warning.

SSE framing handles split UTF-8 and CRLF delimiters, bounds partial frames and
retained events, and advances its durable sequence before a React state update.
The host does not apply its short upstream deadline to that stream: aborting a
live response resets the browser connection, and on HTTP/3 the reset is reported
as a network error. The conversation service cycles the stream itself.
Reconnects use bounded backoff; access rejection ends the stream loop. API reads
and each stream reconnect still renew channel authority through the existing BFF.
See [Chat recovery](../ui/chat-recovery.md) for the transport decision and
[consumer recovery tests](../../apps/chat-ui/src/server/client-recovery.test.ts).

## Sources

- [Channel configuration](../../crates/server/src/domains/agent_channels/record/work_chat.rs)
- [Effective features](../../crates/server/src/domains/agent_channels/work_chat.rs)
- [Consumer routes](../../crates/server/src/api/agent_api/work_chat/mod.rs)
- [Consumer delegation](../../crates/server/src/worker_link/consumer_delegation.rs)
- [Standalone browser host](../../apps/chat-ui/)
- [Execution API](agent-execution-api.md)
- [Virtual users](../runtime-resources/virtual-users.md)
- [Coordination](../runtime-resources/coordination.md)

## Rich worker experience

The browser host renders Markdown together with structured OpenUI and A2UI blocks,
using the [shared generative-UI renderer](../ui/a2ui.md) rather than executing
generated JavaScript. Both chat hosts use the same 69-component A2UI 1.0 catalog,
OpenUI actions and legacy tree adapter. Component colors follow the host theme. Widget
message actions use the consumer's existing conversation and execution policy.
Read-only shares disable those actions. Malformed widgets fall back to source.

Work logs belong to individual replies. Task chips link to the existing delegated
sessions. Conversation search, pagination, pinning, renaming and archive/reopen
operate within the caller's channel history. The permanent main conversation
cannot be archived or renamed through consumer settings. Archived conversations
remain readable and require reopening before another message.

Messages use the model configured on the Agent. A caller cannot select a
different model. Message controls accept typed reasoning effort only; other
execution channels keep ignoring caller controls. Existing budgets and approval
rules apply, with no new rollout flags.

The composer accepts up to three files with a combined four-megabyte limit,
including paste and drag-and-drop. Workspace previews support Markdown, raster
images, searchable CSV and text. HTML/SVG previews run without scripts in an
opaque iframe whose content policy denies network access. Editing retains the
existing revision check; downloading is available for text and binary files.

## Published conversations

A consumer can explicitly publish the messages and structured widgets of one
owned conversation. The random link grants read authority only, never session,
workspace, tool, question or approval authority. Shares remain bound to the live
channel; revocation and replacement serialize against the session. History is
paged and rate limited. Public views use the channel's safe message projection,
exclude tool output and routing metadata, and do not cache responses.

Publishing is a user action, not the default visibility of consumer history.
Source: [experience routes](../../crates/server/src/api/agent_api/work_chat/experience.rs).
