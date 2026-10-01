---
type: Specification
title: "Tool Approval"
description: "The hard, per-call approval gate, and how hosted sessions park a turn on it durably until a person answers."
tags:
  - everruns
  - execution
  - capabilities
  - safety
---

# Tool Approval

## Purpose

`tool_approval` is the hard gate: a risky tool call does not run until a person
says yes. It is the enforced counterpart of [soft approval](soft-approval.md),
which only asks the model to pause. Which calls count as risky comes from the
tool's own `ToolHints` and the configured mode (see
[capabilities](capabilities.md#toolapproval)).

This spec owns how a *hosted* session services the gate (EVE-1140): where the
request is parked, what survives a restart, what a person can answer, and what
happens when nobody does. The in-process contract (a host-supplied approver that
blocks until it answers) is unchanged and lives with the capability.

## Sources of truth

- [`crates/builtins/src/tool_approval.rs`](../../crates/builtins/src/tool_approval.rs)
  owns classification, the decision vocabulary, the hook, the durable approver,
  the stored record, the fingerprint, and the config schema.
- [`crates/provider/src/tool_approval_types.rs`](../../crates/provider/src/tool_approval_types.rs)
  owns the parked-call payload and the synthetic request call the engine emits.
- [`crates/engine/src/execution/act_hooks.rs`](../../crates/engine/src/execution/act_hooks.rs)
  (`ToolApprovalPauseHook`) and `plan_after_act` in
  [`crates/engine/src/turn.rs`](../../crates/engine/src/turn.rs) own the pause.
- [`crates/server/src/api/tool_approvals.rs`](../../crates/server/src/api/tool_approvals.rs)
  owns the answer endpoint and the shared resolution; the deadline pass lives in
  [`crates/server/src/tool_result_timeout.rs`](../../crates/server/src/tool_result_timeout.rs).
- [`docs/api/openapi.json`](../../docs/api/openapi.json) is the wire contract.
- [`crates/server/tests/contracts/tool_approvals_test.rs`](../../crates/server/tests/contracts/tool_approvals_test.rs)
  covers the acceptance paths end to end.

## Why a hosted gate cannot block

An in-process host (the ACP server in `serve`) answers by blocking the turn on
a channel until its client replies. A hosted turn runs as durable activities on
whichever worker claims them; the process that asked may be gone by the time a
person answers, and a worker slot held open for minutes is capacity nobody gets
back. So the hosted approver never waits. It answers from what has already been
decided, and otherwise reports `Deferred`, and the turn parks the same way a
client-side tool or an MCP elicitation parks it.

## Lifecycle

1. The gate classifies the call. If it is gated, the durable approver looks for
   a decision in session storage: an "always" rule for the tool, then a one-off
   answer for this exact call.
2. With no decision, the call does not run. It completes with a structured
   `tool_approval_required` failure the model reads, carrying everything a
   person needs: tool, a bounded preview of the arguments, the reason it was
   gated, and the deadline.
3. The engine's pause hook turns each such result into a synthetic
   `approve_tool_call` call in the act's single `tool.call_requested` batch. Its
   id is derived from the gated call's id, so a replayed act re-derives the same
   request rather than raising a second one.
4. The planner parks the turn. Unlike every other pause, this one does not
   depend on a client hint: the gated call already failed closed, and an API
   caller can answer without drawing anything.
5. A person answers through `POST /v1/sessions/{id}/tool-approvals`. The
   decision is recorded in session storage, the synthetic calls are completed
   (closing the cards), the decision goes in as a user turn (the synthetic call
   is engine-authored, so nothing in the transcript claims its result), and the
   turn resumes.
6. The model calls the tool again. The gate, on whichever worker runs it, finds
   the recorded decision and lets that call through or blocks it.

The retry is deliberate. The gated call already has a result in the transcript,
and resuming a turn into a tool execution the model did not just request is not
something the durable turn loop does; the same retry shape already carries URL
and form elicitation answers. The binding below is what keeps the retry safe.

The OpenAI Agents API backend is the exception: its provider is still waiting on
the original call, so the backend holds that call open, and on an approval runs
it again itself under a fresh local id, through the same gate and binding. See
[OpenAI Agents API runtime](openai-agents-api-runtime.md#policy-at-the-tool-and-output-boundaries).

## Decisions

The vocabulary is the capability's `ApprovalDecision`. A person answers one
of the four allow/reject decisions; the host alone produces the others. A
one-off answer is recorded against the exact call; an "always" answer is a rule
for every call of that tool in the session. A rejection blocks the identical
retry rather than asking again. Cancellation, an unreachable store and "no
decision yet" record nothing: the first two block, the last parks the turn.

A one-off answer is bound to a fingerprint over the tool name and the exact
arguments — nothing ignored, nothing whitespace-normalized, only key order
canonicalized — so approving one call never approves a different one. It is
taken with a destructive read, so concurrent retries cannot spend one approval
twice, and it lapses unused after an hour so a stale yes cannot surface much
later in the session.

The turn resumes once per batch, so every request in it is settled by one
submission. A request left out of the submission is *not approved*, and
nothing is recorded for it: silence never approves, and a retry simply asks
again.

## Timeout

Each request carries a deadline, configurable per agent (`timeout_seconds`,
default 15 minutes, bounded to one minute through one day). When it passes, the
tool-result sweep resolves the batch as not approved through the same
resolution the API uses, so a person answering at the same instant and the
sweep race on one claim and the first writer wins. Nothing is recorded, and a
late answer is refused. A session parked on a request with a deadline is never
resolved by the generic client-tool timeout.

## Security invariants

- The approval is only as strong as who can write it. Decisions live under the
  `tool_approval/` session-storage prefix, which is reserved from the
  model-facing `kv_store` tool and the storage listing, and only the answer API
  writes it. THREAT[TM-TOOL-008].
- What was approved is read from the engine-emitted request, never from the
  submission. Only engine-authored requests count (name, id prefix and payload
  discriminator all match), so a model-authored client tool named
  `approve_tool_call` cannot be answered as an approval.
- Answering takes session-manage authority and the Platform Chat owner check
  every other parked-turn answer takes.
- Fail closed everywhere: no storage, a storage error, a cancelled call, an
  expired deadline or an omitted request all leave the call blocked.
- The gate's in-memory "always" cache is skipped for the durable approver, so a
  long-lived worker serving many sessions does not grow it without bound.

## Not covered yet

- MCP Apps, Slack and A2A surfaces do not render approval cards; those sessions
  can still be answered through the API.
- Per-action gating for computer use (`action_requires_approval`) is a policy on
  top of this gate and ships with the computer use work.
