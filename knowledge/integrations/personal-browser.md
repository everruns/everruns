---
type: Design
title: "Personal browser pairing and handoff"
description: "Session-owned Browse supervisor and authority-backed durable dispatch bridge."
tags:
  - integrations
  - security
---
# Personal browser pairing and handoff

Status: partial implementation. The opt-in `personal-browser` feature supplies
a persistent session-owned subprocess supervisor, narrowly exposed browser tools
and an authority-backed durable dispatch coordinator. Yolop's host adapter and
terminal handoff guidance are under separate local validation. Actual extension
pairing, lending and human Continue and stopping UX still require end-to-end evidence.
The isolated direct-CDP backend is separate from personal profiles. No release,
deployment or extension installation is authorized by this implementation.

## Reuse upstream Browse, not a new extension

Investigated Browse commit
[`c3b29be`](https://github.com/anomalyco/browse/tree/c3b29be5299658281fe4ecdcb926bb7b5a9d6f8c).
Its native MCP process talks through a token-protected relay to the extension;
the extension owns Chrome debugger attachment, pairing, lending/revocation, and
human continuation. Reuse that consent surface. An Everruns approval alone must
not substitute for the user's browser-side consent or successful completion.

Browse's remote relay mode connects the extension to the agent's relay. It is
not a generic remote MCP endpoint or sandbox CDP transport. Keep personal and
sandbox browser modes separate, with explicit host policy and credentials.

## Existing Framework building blocks and the transport gap

* `Session::add_mcp_server` (`crates/everruns/src/session/controls.rs`) provides
  trusted, session-scoped MCP configuration and discovery/readiness controls.
  Configuration is not serialized with the session. Reattachment on restart is
  the host's responsibility.
* `ClientSideToolHook` (`crates/core/src/engine/execution/act_hooks.rs`) emits
  durable `tool.call_requested` waits. `Session::pending_tools` and
  `Session::resolve_tools(command_uuid, turn_id, answers)`
  (`crates/everruns/src/session.rs`) provide inspection and idempotent resolution.
  Reuse these rather than introduce a second execution/wait engine.
* Current `StdioTransport::run` (`crates/core/src/mcp/stdio.rs`) starts a new
  child for each operation; discovery also starts a separate child. Simply
  configuring Browse as a stdio server does **not** preserve its owner identity
  and tab continuity across calls.
* Browse [`mcp.rs`](https://github.com/anomalyco/browse/blob/c3b29be5299658281fe4ecdcb926bb7b5a9d6f8c/rust/crates/browse/src/mcp.rs)
  defaults ownership to the MCP connection. Its OpenCode session metadata path
  hashes a session ID to only 32 bits. Do not use that identifier as a tenant
  authorization boundary or pool unrelated sessions through it.

## Adapter boundaries

1. A host-owned `BrowseSupervisor` starts one persistent pinned
   Browse MCP process per Everruns session. Treat this as a new lifecycle-aware
   transport or broker, not a command wrapper around the existing per-call
   transport. Bound process count, output, queue length, and timeouts. Route MCP
   requests/cancellation correctly; serialize lifecycle transitions. Scope the
   broker's authenticated caller to its session, not a caller-provided owner ID.
2. Expose pairing/readiness/status to Framework and Yolop with the same adapter.
   Reuse upstream extension installation and pairing instructions. Never install
   into a personal profile silently. Keep pairing tokens in host credential
   storage, redact logs, and never put tokens in model-visible tool results.
3. Present handoff as a client-side tool, `personal_browser_handoff`, with
   an owned tab reference and human-readable reason. Park it using existing
   durable tool machinery. A host worker starts the upstream handoff and resolves
   the exact pending call only after the extension reports its outcome.
4. The host calls `enable_session`, `run_pending` and `close_session`; these
   manage session exposure, durable reconciliation and process cleanup, not
   extension pairing. Authority records retain session/tool-call/turn identity,
   argument fingerprint, owner/fence, deadline and terminal result. They never
   retain browser cookies or pairing tokens. Host routes require authentication;
   completion uses the stable resolution UUID and rejects stale responses.
5. Preserve the extension's real Continue interaction. The pinned card has no Cancel button; stopping uses upstream revocation/cancellation paths. A Framework/Yolop
   status panel may show the wait, cancel it, or help the user find the tab; a
   UI button must not claim browser-side work finished without verified outcome.
   After handoff, re-observe page state and invalidate stale references before
   executing further browser actions.

## Durability boundary

A durable Everruns wait is not a durable browser action. Browse's
[`serve.rs`](https://github.com/anomalyco/browse/blob/c3b29be5299658281fe4ecdcb926bb7b5a9d6f8c/rust/crates/browse-relay/src/serve.rs)
cleans up calls and handoffs when their owning process exits. Its handoff is an
in-process awaited operation, not a resumable transaction.

Framework restart may preserve the pending wait while its process-local
coordinator is lost. Broker/relay restart must fail closed: mark the attempt
interrupted, reacquire browser consent as needed, and inspect state. Never replay
an unknown-outcome click, submit, or purchase automatically. True resume across
broker death requires an upstream resumable handoff protocol or an explicitly
specified broker protocol; it is outside this minimal phase.

## Required acceptance before claiming personal-browser support

* Two sessions have distinct persistent owners for agent-created tabs. User-lent
  tabs are shared across connected agents in the same upstream Browse profile;
  personal mode is a single-user trust domain, not tenant isolation. Sequential
  calls preserve the intended owner. Test foreign agent-created tab denial.
* Pairing denial and tab revoke fail closed, including revoke during execution.
* Genuine user Continue, stopping, timeout, and tab closure produce distinct
  terminal results, with exactly-once durable resolution under retry.
* Framework restart retains the pending wait; broker restart reports interrupted
  and does not replay mutations. Reconnect never silently adopts another tab.
* Cancellation kills only the correct request/session resources. Supervisor
  shutdown cleans up children and respects external browser ownership.
* Tokens, profile data, and authenticated page contents do not leak in logs,
  cross-session artifacts, or default telemetry.
* Framework and Yolop expose equivalent pairing/wait states. Use a dedicated
  test Chrome profile and synthetic pages; personal extension testing requires
  an explicit user installation/pairing step.

Implement and review this trust boundary separately from the direct-CDP foundation. The direct
CDP acceptance fixture is useful infrastructure but is not evidence that these
personal-browser cases pass. This design does not require a new release; Yolop
package/version coordination remains separate from local path-based validation.

## Implemented host primitive

`everruns::personal_browser` reexports `BrowseSupervisor`, `BrowseProcessConfig`,
`BrowseState`, `BrowseError` and `HandoffOutcome`. The host supplies an absolute
path to its pinned upstream executable and starts one supervisor with the
Framework session's `session_id().uuid()`. Every tool/handoff request checks
that owner; model input cannot set MCP metadata or executable configuration.
The library preserves one initialized stdio process rather than spawning per RPC.
Requests, response frames, queue depth and time are bounded. Drop/cancel/timeout
ends the owner process instead of automatically repeating a possibly completed
browser action. Remote errors are redacted; page results still require private
session-scoped handling. Hosts must cap total supervisor count and authenticate
all user-facing routes. No extension is installed and no personal profile is
opened by this library automatically.

The supervisor itself is not a durable workflow; the separate coordinator
provides authority-backed dispatch. Neither synthetic protocol tests nor real
MCP discovery establishes extension pairing, tab-lending or human UX.

## Durable dispatch authority

The existing client-tool wait is durable, but dispatching work to an external
process was not previously an atomic authority operation. `Engine::session_status`
returns the open turn and waiting calls together; `resolve_tools` is idempotent
and turn-bound. The new dispatch commands add the missing atomic claim before
external execution.
Generic session KV has no atomic claim/CAS, and epoch writes require the agent's
active execution lease, which a parked external worker should not hold.

The bridge uses generic authority-backed client dispatch commands, not a
browser-specific file journal:

1. Begin dispatch validates the exact pending session/turn/call/name/arguments
   and atomically records the request fingerprint, worker incarnation/fence,
   deadline and stable resolution UUID. Only one owner may dispatch.
2. Complete dispatch validates that claim, rejects stale/conflicting completion,
   and records terminal outcome plus tool resolution in one authority
   transaction. Retries retain the same command identity and response.
3. A lost/expired worker becomes interrupted/unknown, never success or automatic
   redispatch. Crash after claim but before send conservatively loses an action
   rather than risk duplicating an unknown mutation.

This is implemented in the shared session command reducer and its SQLite, memory and
cell authority adapters using existing framework objects and receipts. The
Framework coordinator then declares narrowly selected upstream schemas as
client tools, binds supervisors by full session ID, dispatches outside the
agent worker, and reconciles retained terminal results without replay. Do not
expose upstream administrative/settings tools blindly, or capture one root
supervisor in an Agent reused by child sessions. Caller-retained IDs with
`create_registered`, or a session-keyed host registry, solve that binding.

Required qualification includes concurrent claims, lost responses, crashes on
both sides of dispatch, stale worker completion, wrong session/turn/call,
duplicate results, restart reconciliation and child-session isolation. The authority extension and coordinator remain separate from the low-level
supervisor.

## Implemented coordinator and host obligations

`PersonalBrowserCoordinator` binds a full session ID to a bounded persistent
supervisor and exposes only status, tab listing, handoff and narrow browser
actions. `enable_session` uses a session configuration overlay, preserving
unrelated client tools and refusing conflicting reserved declarations. It does
not implicitly enable children. `declare_tools` is only for hosts that service
every session created from that Agent. The host still authenticates all session
routing; owner UUIDs are fencing identities, not credentials.

The host reconciles calls with `run_pending` outside an execution lease, on new
waits and restart, and at returned peer-claim deadlines. Claims are never
reassigned or replayed. Atomic completion persists the terminal result, receipt
and continuation; retry after a lost completion response also reconciles engine
activation. The memory adapter appends before synchronously changing its records,
so cancellation while awaiting append cannot leave a partial completion.

Call `close_session` to interrupt initialization or an active handoff and reap
only that session's process. A coordinator has a bounded session registry and
closed bindings are not silently reopened. Browser state after interruption
must be inspected before deciding on any fresh action. Screenshot results become
image content; bounded oversized results resolve as explicit failures instead
of stranding the durable claim. Page content remains untrusted private data.

Authority tests and protocol fixtures cover failure/retry boundaries separately
from real-extension UX. Record actual end-to-end results before claiming full
pairing, lending, revocation or human-interaction qualification.
