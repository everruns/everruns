---
type: Specification
title: "Ask User"
description: "Structured choice questions resolved by client-side or in-process hosts."
tags:
  - everruns
  - execution
---
# Ask User

## Purpose

`ask_user` lets a model collect one small batch of structured decisions without
ending the conversation turn in prose. Hosted clients use the existing
client-side tool lifecycle. An in-process Framework host resolves the same
contract inside the tool call.

This capability is for decisions and preferences. It is never a consent gate.
Permission for a destructive, irreversible, or outward-facing action uses
`request_approval`, whose wait does not auto-resolve.

## Sources of truth

- [`crates/builtins/src/ask_user.rs`](../../crates/builtins/src/ask_user.rs)
  owns the request and result types, exact input schema, validation, default
  materialization, prompt guidance, host responder trait, and both execution
  strategies.
- [`crates/everruns/src/ask_user.rs`](../../crates/everruns/src/ask_user.rs)
  projects the responder contract at its stable Framework path.
- [`crates/provider/src/tool_types.rs`](../../crates/provider/src/tool_types.rs)
  owns the client-side tool-definition wire contract.
- [`crates/engine/src/execution/act.rs`](../../crates/engine/src/execution/act.rs)
  owns client-call partitioning and the act pause boundary.
- [`crates/engine/src/execution/act_hooks.rs`](../../crates/engine/src/execution/act_hooks.rs)
  owns request-event emission and waiting-state signaling.
- [`crates/server/src/api/tool_results.rs`](../../crates/server/src/api/tool_results.rs)
  owns result persistence and durable workflow resume.
- [`client-side-tools.md`](client-side-tools.md) owns the shared lifecycle,
  abandonment, timeout recovery, and security rules.

Exact fields and limits stay in source and generated tool schemas rather than
being copied here.

## Contract decisions

One call batches related questions into one host interaction. The client-side
strategy pauses once. EVE-1053 supports choice questions only; secret
collection is a separate capability extension.

Question identifiers are stable result-correlation keys. The runtime preserves
an identifier supplied by the model and generates a collision-free identifier
when it is absent.

A single-select question may mark zero or one default. When no default is
marked, timeout resolution chooses the first option, so the model orders options
most-applicable-first. A multi-select question may mark more than one default.
The model may shorten the declared timeout but cannot extend it past the
platform ceiling.

The result distinguishes answered, declined, cancelled, and timed-out
decisions. A decline is final and must not be re-asked. The result also says
whether a user, timeout, or unattended policy supplied the answer; it does not
carry user identity. Authenticated server-side handling owns attribution.

## Execution strategies

`AskUserCapability::client_side()` contributes a client-side definition. The
hosted product uses this strategy so a browser can answer after the current
worker turn parks.

`AskUserCapability::new(responder)` contributes a built-in tool. The tool
awaits the host responder in-process and returns its outcome directly to the
model without entering `waiting_for_tool_results`.

The default in-process capability uses `DefaultsResponder`. It selects marked
defaults, or the first option when no default is marked, and reports
`answered_by: "unattended"`. This keeps tests and headless runs from hanging.

## Client-side pause and resume

The client-side strategy contributes no server-side tool. Its definition is
never deferred behind tool search. When the model calls it, the act atom follows
the standard client-side path:

1. preserve the model-authored call in the assistant transcript;
2. normalize omitted contract defaults and question identifiers;
3. emit the client-tool request and set the waiting-for-tool-results state;
4. accept the correlated result through the existing tool-results endpoint;
5. persist an ordinary tool completion and resume the durable turn.

The result alone returns the answer to the model. No synthetic user message is
needed because the model authored the call.

Whether the turn parks on step 3 at all is a client-capability question, so it
rides a session hint — `ask_user`, declared by the UI alongside
`setup_connection` and `url_elicitation`, the same mechanism URL elicitation
uses. A client that never declared it (a scheduled run, a trigger, an SDK
caller) has nobody to answer the card, so the planner answers the call itself
with the model's declared defaults and `answered_by: "unattended"` in the same
turn rather than burning the whole timeout on a human who is not there. One
rule covers every headless surface. The engine recognises the call through
`ASK_USER_TOOL_NAME` in `everruns-provider`, and `unattended_ask_user_result`
there is the JSON twin of `DefaultsResponder`; a drift test in
`everruns-builtins` fails if the two disagree.

Deadline timestamp generation and automatic timeout resolution are separate
work. This contract carries the bounded timeout duration, but it does not add
server ticks or deadline timestamps to the emitted call.

## Safety invariants

- The tool declares itself read-only and non-destructive.
- Automatic timeout defaults cannot authorize risky actions.
- Consent remains on `request_approval`; `ask_user` must not replace it.
- The model inspects answer provenance before acting on a timed-out or
  unattended choice.
- A declined choice is a completed decision, not a prompt to ask again.
