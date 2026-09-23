---
type: Specification
title: "Ask User"
description: "Structured choice and credential questions resolved by client-side or in-process hosts."
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
strategy pauses once. Two kinds exist: `choice` offers options, and `secret`
collects one credential (see below).

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
uses. An MCP client declares it by declaring `elicitation`, and is then asked
the question set as a form mode elicitation ([mcp.md](../integrations/mcp.md)).
A client that never declared it (a scheduled run, a trigger, an SDK
caller) has nobody to answer the card, so the planner answers the call itself
with the model's declared defaults and `answered_by: "unattended"` in the same
turn rather than burning the whole timeout on a human who is not there. One
rule covers every headless surface. The engine recognises the call through
`ASK_USER_TOOL_NAME` in `everruns-provider`, and `unattended_ask_user_result`
there is the JSON twin of `DefaultsResponder`; a drift test in
`everruns-builtins` fails if the two disagree.

### Deadlines

Normalization stamps `asked_at`, `nudge_at` and `expires_at` onto the call
(EVE-1056). They are server-owned: whatever the model supplied is discarded and
replaced, because `expires_at` is the instant the server stops waiting, and a
caller that chose its own would be choosing when it stops waiting for a human.
The parameters schema declares all three `readOnly` — it is
`additionalProperties: false` and validates the *normalized* call, so a stamped
field it does not declare gets the whole call rejected before the tool runs.

`nudge_at` keeps its shipped shape at the default 300s timeout — 60s before
expiry, the four-minute mark — but scales below that, because a fixed 60s lead
on a short window would land at or before `asked_at` and open the card already
warning. Surfaces render the deadline the server will act on rather than one
they derived, so the countdown and the resolution cannot disagree.

At `expires_at` the tool-result sweep resolves the call with each question's
declared default (or its first option), `status: "timed_out"` and
`answered_by: "timeout"`. Both fields matter: the value is a default being
applied, not consent, and only `answered_by` says so. A secret question resolves
`declined` instead — a credential has no default worth applying.

The sweep keeps its 30s periodic shape rather than per-session timers, so a
deadline survives a restart; firing up to 30s late is acceptable, losing it on
deploy is not. It resolves through the shared resolution operation rather than
writing a completion event directly, so a human answering at the same instant
races it on one claim and the first writer wins.

A session parked on `ask_user` never falls through to the generic
`waiting_for_tool_results` timeout, whose payload would tell the model the
client went away when in fact nobody answered a question. A call recorded before
the server stamped deadlines carries no `expires_at`, and is left to that
generic path rather than resolved on a deadline nobody wrote down.

## Secret questions

`kind: "secret"` collects a credential in flow. It exists because an `ask_user`
answer is a *tool result*: a credential typed into an ordinary question box
would be plaintext in the event log **and** replayed into model context every
turn, which is strictly worse than one typed in chat. It closes the in-flow half
of TM-AGENT-016.

The answer returns a reference, never a value. There is no `value` field on
`AskUserAnswer` — not empty, absent — so no code path carries a collected
secret into a result. The client stores the value through the session-secret
endpoint that has always encrypted it, then answers the question with
`session:{secret_name}`; tools already resolve session secrets by name. Two
posts, one card, and no endpoint that could persist the value as a result.
Resolution refuses an answer carrying a selection or free text on a secret
question rather than ignoring it, and refuses a reference to a secret that was
not the one asked for — the caller does not get to say what it was asked, the
same rule the choice path applies to option labels. It also refuses a reference
to a secret that is not actually stored, because a handle resolving to nothing
reads to the model as answered.

Three shape rules follow from that and are enforced in validation or
normalization:

- a secret question is the only question in its call, so the unattended outcome
  below is unambiguous and the card stays a password field rather than a form;
- `secret_name` and `purpose` are required — nobody should type a credential
  without being told what it is for;
- free text and multi-select are normalized off, because free text is exactly
  the path a typed credential would take into the result.

A secret question never auto-resolves. A "default credential" is meaningless, so
`DefaultsResponder` and the engine's unattended path both decline the call
rather than answer it, and proceeding without the credential becomes the model's
explicit decision. This overrides the deadline-defaults rule in EVE-1056.

Surfaces project this rather than rebuild it. `/mcp` never puts a secret
question in the form mode elicitation it uses for choice questions
([mcp.md](../integrations/mcp.md)) — it already has
`ElicitationIntent::SessionSecret` — a signed, principal-bound token and a form
writing through `BatchSetSessionSecrets`. A2A refuses outright: a remote agent is
never handed a prompt for a human's credential, so a secret question projects as
`auth_required` carrying a URL, never a `DataPart` asking for the value. Forking
needs no work; `session_secrets` copy ciphertext verbatim, so a `secret_ref`
survives a fork.

## Safety invariants

- The tool declares itself read-only and non-destructive.
- Automatic timeout defaults cannot authorize risky actions.
- Consent remains on `request_approval`; `ask_user` must not replace it.
- The model inspects answer provenance before acting on a timed-out or
  unattended choice.
- A declined choice is a completed decision, not a prompt to ask again.
- A secret answer carries a reference; the value reaches only the encrypted
  session-secret store, never an event, tool result, or model context.
- A secret question has no unattended or timeout answer.
