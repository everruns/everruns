---
type: Decision
title: "Chat Recovery and Live Transport"
description: "Keep resumable SSE while recovering tab-local intent and sharing session subscriptions without conflating replay cursors."
tags:
  - everruns
  - ui
  - streaming
  - recovery
---

# Chat Recovery and Live Transport

## Decision

Keep HTTP commands and resumable SSE for Chat, Work Chat and the Agent Execution
API. Offline resilience depends on preserving intent and reconciling delivery;
changing the socket supplies neither. A production WebSocket transport needs
evidence of frequent bidirectional traffic or material improvement over HTTP
connection reuse. Provider WebSockets are a separate hop; see
[OpenAI Responses WebSocket Transport](../foundations/openai-responses-websocket.md).

## Recovery contract

Drafts and unacknowledged sends survive navigation and reload in the same tab.
Recovery is bound to account, organization and session. Uploaded attachment
references are preserved; raw files still waiting for upload are not stored.
Credentials are not written to recovery records.

A connectivity failure in a live tab may retry on network return with its
original identity. Console sends use the existing atomic command endpoint with
that identity as its idempotency key. The legacy REST handler's recent-input
lookup is not an atomic concurrent-send guarantee. Retries stop after 23 hours,
inside the command store's 24-hour retention, measured from the original send;
retrying never extends this deadline. Expired local copies remain reviewable
and discardable. Automatic recovery is serial and stops at a failed retry.
Server rejections do not become connectivity retries. Restored sends require
explicit Retry, or Discard of the local copy. Reload is not permission to
execute an old instruction. Discard does not cancel work already accepted by
the server. Approvals and cancellation are not queued.

Acceptance retires the persisted send before its event echo. Durable events
own accepted work. Late results cannot mutate a replacement scope. Logout
clears recovery, including embedder-provided logout. This is browser-local
plaintext subject to same-origin access, not an encrypted secret store.

Storage is bounded. Corruption, disabled storage and quota failures keep text
editable and warn users to preserve it before leaving. They never claim
successful local persistence. Closing a tab or clearing site data can remove
recovery. Duplicated tabs retain copied message identities.

This does not provide an offline application shell, an offline model, or cached
transcripts after an offline cold reload. Sources:
[storage](../../apps/ui/src/lib/chat-recovery.ts),
[drafts](../../apps/ui/src/hooks/use-chat-draft.ts), and
[sends](../../apps/ui/src/hooks/use-chat-sends.ts).

## Hosted Work Chat

The standalone consumer host also recovers drafts and unconfirmed requests,
using a non-credential browser-login namespace and the Agent API idempotency
contract. It requires explicit Retry and keeps retries inside that contract's
retention window. Its attachment bytes are part of the saved bounded request;
large attachments can exceed browser quota and produce the storage warning.
See [Work Chat channels](../integrations/work-chat.md) for its authority and
streaming boundaries.

## Subscriptions and coordinator work

Snapshot consumers share a session stream with a transcript within one org and
authenticated query cache. A task-first stream reopens from the transcript's
cursor; snapshot consumers refresh on connection. Independently loaded
transcripts keep independent streams to preserve replay. Different filters,
origins and credential modes never share. Logout disposes shared transports;
queued old callbacks are ignored.

Online, focus and visibility return can restart a disconnected stream after
retry exhaustion. Healthy streams remain open. Existing opted-in idle
suspension is preserved. Observed task and turn transitions coalesce authorized
thread-list refreshes. Polling remains for work outside observed sessions;
this change does not introduce an organization-wide event feed.

Sources: [transport](../../apps/ui/src/lib/event-stream.ts),
[session events](../../apps/ui/src/hooks/use-sessions.ts),
[tasks](../../apps/ui/src/hooks/use-session-tasks.ts), and
[thread lists](../../apps/ui/src/hooks/use-chat-threads.ts).

## Evidence and performance boundary

[bench-chat-transport.mjs](../../scripts/bench-chat-transport.mjs) exercises the
actual UI sharing implementation and a transport-only fixture with equal
command, acknowledgment and completion payloads. HTTP/1.1 keep-alive plus SSE
and uncompressed WS receive the same injected round-trip delay. TLS, HTTP/2,
production proxies, persistence, inference and worker scheduling are excluded.
Wire totals include setup and warmup. PR evidence records samples and environment;
the fixture cannot establish a production speed claim.

Twenty transcript/task pairs hold twenty connections instead of forty.
Task-first mounting requires a promotion handshake. Lower WS framing overhead
on tiny commands is expected, but does not prove an offline or production
latency advantage. Compression and delta batching are separate choices.

Regression proof:
[storage tests](../../apps/ui/src/lib/__tests__/chat-recovery.test.ts),
[send tests](../../apps/ui/src/hooks/__tests__/use-chat-sends.test.ts),
[draft tests](../../apps/ui/src/hooks/__tests__/use-chat-draft.test.ts),
[sharing tests](../../apps/ui/src/lib/__tests__/shared-event-stream.test.ts), and
[manual recovery case](../test-cases/ui/chats/TC019_stream_reconnect_and_cancel.md).

## Revisit

An authorized cross-session summary feed can replace external-work polling,
but must preserve per-user visibility and recover gaps. Frequent interactive
tools, collaboration or steering may justify optional WS on existing execution
services. Neither should fork runtime semantics or replace resumable HTTP APIs.
