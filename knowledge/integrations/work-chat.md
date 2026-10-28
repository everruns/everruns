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
reach consumer session and self-service routes only. Organization membership is
an optional execution auth method. Identity providers remain extensible through
the existing channel verifier and execution configuration.

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
vendor or cloud domain is hardcoded upstream. Channel identifiers supply stable
DNS-safe labels. The hostname, never a browser-supplied API URL or org, selects
the channel.

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
Reconnects use bounded backoff; access rejection ends the stream loop. API reads
and each stream reconnect still renew channel authority through the existing BFF.
See [Chat recovery](../ui/chat-recovery.md) for the transport decision and
[consumer recovery tests](../../apps/chat-ui/src/server/client-recovery.test.ts).

## Sources

- [Channel configuration](../../crates/server/src/domains/agent_channels/record/work_chat.rs)
- [Effective features](../../crates/server/src/domains/agent_channels/work_chat.rs)
- [Consumer routes](../../crates/server/src/api/agent_api/work_chat.rs)
- [Consumer delegation](../../crates/server/src/worker_link/consumer_delegation.rs)
- [Standalone browser host](../../apps/chat-ui/)
- [Execution API](agent-execution-api.md)
- [Virtual users](../runtime-resources/virtual-users.md)
- [Coordination](../runtime-resources/coordination.md)
