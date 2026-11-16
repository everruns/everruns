---
type: Design
title: "Personal browser pairing and handoff"
description: "Proposed session-owned Browse adapter and durable human handoff boundary."
tags:
  - integrations
  - security
---
# Personal browser pairing and handoff (proposed)

Status: partial implementation. The opt-in `personal-browser` feature now supplies
a persistent session-owned subprocess supervisor and typed upstream handoff
outcomes. The Framework durable bridge, Yolop UI and actual extension workflow
remain **not implemented or verified**. The direct-browser
foundation controls isolated Chromium contexts through host-configured CDP. It
does not provide personal-profile access, extension pairing, tab lending, or
human handoff. No release is required or authorized by this design.

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

## Proposed minimal adapter

1. A host-owned `PersonalBrowserSession` supervisor starts one persistent pinned
   Browse MCP process per Everruns session. Treat this as a new lifecycle-aware
   transport or broker, not a command wrapper around the existing per-call
   transport. Bound process count, output, queue length, and timeouts. Route MCP
   requests/cancellation correctly; serialize lifecycle transitions. Scope the
   broker's authenticated caller to its session, not a caller-provided owner ID.
2. Expose pairing/readiness/status to Framework and Yolop with the same adapter.
   Reuse upstream extension installation and pairing instructions. Never install
   into a personal profile silently. Keep pairing tokens in host credential
   storage, redact logs, and never put tokens in model-visible tool results.
3. Present handoff as a client-side tool, for example `browser_handoff`, with
   an owned tab reference and human-readable reason. Park it using existing
   durable tool machinery. A host worker starts the upstream handoff and resolves
   the exact pending call only after the extension reports its outcome.
4. Proposed host-facing operations are `pairing_status`, `begin_handoff`,
   `handoff_status`, and `cancel_handoff`; these are **not existing APIs**.
   Persist an opaque handoff ID, session/tool-call/turn identity, tab binding,
   lifecycle state, and terminal reason. Never persist browser cookies or tokens
   in the wait record. Authenticate result submission; bind it to the outstanding
   handoff, reject stale responses, and reuse the resolution command UUID on
   retries. Distinguish completed, cancelled, revoked, timed out, and interrupted.
5. Preserve the extension's real Continue/Cancel interaction. A Framework/Yolop
   status panel may show the wait, cancel it, or help the user find the tab; a
   UI button must not claim browser-side work finished without verified outcome.
   After handoff, re-observe page state and invalidate stale references before
   executing further browser actions.

## Durability boundary

A durable Everruns wait is not a durable browser action. Browse's
[`serve.rs`](https://github.com/anomalyco/browse/blob/c3b29be5299658281fe4ecdcb926bb7b5a9d6f8c/rust/crates/browse-relay/src/serve.rs)
cleans up calls and handoffs when their owning process exits. Its handoff is an
in-process awaited operation, not a resumable transaction.

Framework restart may preserve the pending wait while an independently owned
broker remains alive. Broker/relay restart must fail closed: mark the attempt
interrupted, reacquire browser consent as needed, and inspect state. Never replay
an unknown-outcome click, submit, or purchase automatically. True resume across
broker death requires an upstream resumable handoff protocol or an explicitly
specified broker protocol; it is outside this minimal phase.

## Required acceptance before claiming personal-browser support

* Two sessions have distinct persistent owners; neither can address, lend, or
  control the other's tab. Sequential calls preserve the intended owner's tab.
* Pairing denial and tab revoke fail closed, including revoke during execution.
* Genuine user Continue, Cancel, timeout, and tab closure produce distinct
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

Implement and review this trust boundary in a separate follow-on PR. The direct
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

The supervisor itself is not a model capability or durable workflow. Do not
advertise it as complete personal-browser integration. Its synthetic protocol
tests do not establish upstream extension pairing, tab-lending or human UX.

## Durable dispatch prerequisite

The existing client-tool wait is durable, but dispatching work to an external
process is not yet an atomic authority operation. `Engine::session_status`
returns the open turn and waiting calls together; `resolve_tools` is idempotent
and turn-bound. Neither claims a pending call before external execution.
Generic session KV has no atomic claim/CAS, and epoch writes require the agent's
active execution lease, which a parked external worker should not hold.

A correct bridge needs generic authority-backed client dispatch commands, not a
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

This belongs in the shared session command reducer and its SQLite, memory and
cell authority adapters using existing framework objects and receipts. The
Framework coordinator then declares narrowly selected upstream schemas as
client tools, binds supervisors by full session ID, dispatches outside the
agent worker, and reconciles retained terminal results without replay. Do not
expose upstream administrative/settings tools blindly, or capture one root
supervisor in an Agent reused by child sessions. Caller-retained IDs with
`create_registered`, or a session-keyed host registry, solve that binding.

Required qualification includes concurrent claims, lost responses, crashes on
both sides of dispatch, stale worker completion, wrong session/turn/call,
duplicate results, restart reconciliation and child-session isolation. This
authority extension and coordinator are not part of the current supervisor.
