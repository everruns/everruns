---
type: Specification
title: "Soft Approval"
description: "Spoken-consent confirmation before critical actions, injected as prompt guidance rather than enforced as a permission gate."
tags:
  - everruns
  - execution
  - capabilities
  - safety
---

# Soft Approval

Status: implemented. Capability `soft_approval`, source
`crates/builtins/src/soft_approval.rs`.

## Why

An agent with a shell, a file system, a network, and platform tools touches
real things: it deletes files, pushes branches, sends messages, drops org-wide
entities. People want a say before the risky ones, but a hard, per-call yes/no
gate is miserable. It interrupts safe work, it cannot be reasoned about, and it
trains the user to approve reflexively, which is worse than no gate at all.

Soft approval takes the opposite tack. It is **prompt-engineering, not a
permission gate**: the agent is told, in its system prompt, to batch the safe
work and to pause for spoken consent only at the genuinely critical moments.
The model decides what is critical, the user approves in plain language, and
the grant is logged. There is no separate approval UI to wire up, consent lives
in the conversation.

## What

### Levels

One setting, `mode`, picks how cautious the agent is. The vocabulary is
[`ApprovalMode`](../../crates/builtins/src/tool_approval.rs), shared with the
hard [`tool_approval`](capabilities.md#toolapproval) gate so a deployment tunes
both layers with one word:

| Level | The agent pauses before… |
|---|---|
| `protective` | any state-changing action |
| `normal` | clearly destructive, irreversible, or outward-facing actions (default) |
| `off` | nothing; the capability contributes no prompt at all |

`ApprovalMode::parse` is lenient about synonyms (`paranoid`, `yolo`) because
the same string arrives from capability config, a host's own settings file, and
the model-facing `set_approval_mode` tool.

### System-prompt injection

The capability resolves the level per turn and contributes a `<soft_approval>`
block: the threshold for that level plus the operating rules. Plan first and
**batch** the safe steps without pausing (read-only inspection never needs
approval); stop before a critical action, briefly justify it and ask one short
question; treat an affirmative reply as the approval; record it; then proceed.
One approval covers the action described, not unrelated later ones, and a
user-granted category exemption ("you don't need to ask for commits") is
honored without re-asking. `off` contributes nothing, so a session that does
not want the layer pays no tokens for it.

Resolving per turn is what makes `set_approval_mode` and a config edit take
effect on the very next turn, with no restart.

### The pause is a tool call

Prose alone cannot express a pause. A model that ends its turn on "Deploying
now." has stopped, but the loop, the transcript, and the user all see what they
would see if it had finished: text, then nothing. The user is left to guess
that the agent is waiting on them, and the usual guess is that the turn broke.

So pausing means calling `request_approval` with the action and the question,
and ending the turn there. The call **is** the pause. It gives hosts a fact to
render (`PendingApprovalStore`, read when a turn ends) and the audit log a
record of what was *asked*, not only of what was granted. `record_approval`
clears the pause, as does the user's next message: either way they have
answered. The prompt names the failure directly, because it is the one models
actually make: never announce a critical action and then stop without the call.

### Recording a grant, and who it belongs to

`record_approval` is called right after the user consents. Because every tool
call already persists a completion event on the session, the approval lands in
that durable log for free, with a specific `action` description plus optional
detail. Empty arguments fall back to a conservative description rather than
failing: a model that emits a bare call after real spoken consent should not
turn a granted approval into an error. Both tools tell the model not to pass
secrets, since the arguments are logged.

**The tools never say who approved.** A model-written `approved_by` is an
assertion by the thing being governed, and an audit trail whose actor field is
supplied by the agent answers nothing. What the tools record is *where* the
consent was spoken: `request_approval` stamps `asked_in_turn`, and
`record_approval` stamps `approved_in_turn` and `approved_in_message`, read
from the turn's `EventContext`. Identity is then resolved by whoever owns the
authenticated record of that message.

In Everruns that resolution is server-side. The API writes the authenticated
caller onto every `input.message` event as `initiator` / `acting_principal_id`
(`execution_metadata::interactive_user_metadata`), and
`ApprovalAuditListener` reads it back for the message the consent was spoken
in. So the approver is the identity that sent that message, established by
authentication rather than by the transcript.

Two consequences worth stating plainly:

- A turn with no human initiator (a schedule, a trigger) resolves to no user.
  The row is still written, marked `actor_resolution: unattributed`. An
  approval nobody human granted is exactly what an auditor wants to see, so it
  is recorded rather than suppressed.
- In a session several people can post to, the grant belongs to whoever sent
  the consenting message, not to the session owner. That is the correct
  attribution, and it is only available because the join runs against the
  message, not the session.

### Audit

Two records, for two different questions.

**The session event log** answers "what happened in this conversation": the
ask and the grant appear in order, in context, and are readable from
`/v1/sessions/{id}/events`. This is the only record a portable host
(`everruns-builtins` alone, or a terminal agent) gets, and for a single-user
host it is sufficient.

**The org audit log** (`audit_logs`, see [Audit Logging](../security/audit-logging.md))
answers "who approved what, across the org, after the session is gone".
`ApprovalAuditListener` writes one row per ask (`agent.approval.requested`) and
per grant (`agent.approval.granted`), with the resolved actor, the session as
target, and the action, detail, question, and correlation ids as details. It
is an `EventListener`, so it is fire-and-forget and never blocks or fails a
turn; a failed actor lookup downgrades the row to unattributed rather than
dropping it. `set_approval_mode` is deliberately not audited here: changing the
level is configuration, not consent.

The audit row carries a **bounded excerpt**, not the full text. `audit_logs` is
read under `AUDIT_LOG_VIEW` by org admins who may hold no access to the session,
and the copied fields are model-authored from the conversation, so each
free-text detail is capped and marked when truncated (TM-OBS-011). The complete
text stays in the session event log under session permissions, and the
correlation ids on the row are what take a reader there.

### Where the level lives

Effective level = session or host override, else capability config, else
`normal`.

The durable level is the agent's capability config. The override is an
[`ApprovalModeStore`](../../crates/builtins/src/soft_approval.rs); the default
implementation is per-session and in-memory, which is exactly as long-lived as
the "be more careful for the rest of this conversation" it exists to hold. A
host whose approval level is its own durable, cross-session setting (a terminal
agent with a settings file) implements the trait instead, and
`set_approval_mode` writes through to that setting. That seam is what lets such
a host adopt this capability without giving up its central setting; it is the
migration path for [yolop](https://github.com/everruns/yolop), whose own
soft-approval layer this capability generalizes.

### Defaults

Enabled at `normal` on both auto-provisioned harnesses that can act:

- **Generic** (`crates/server/src/harnesses/generic.rs`): shell, file system,
  and network, so an unattended agent can delete or publish for real.
- **Platform Chat** (`crates/server/src/harnesses/platform_chat.rs`): platform
  tools that create, mutate, and delete entities for a whole organization. Its
  prompt keeps the platform-specific confirmation cases (creating a harness or
  agent) and defers the mechanics of pausing to `request_approval`.

`base` stays bare, as it is a parent for composition rather than a harness that
acts. Any agent overrides the level, including to `off`.

## Composition

- **Not a hard gate.** Soft approval cannot *prevent* a tool call; it asks the
  model to. [`tool_approval`](capabilities.md#toolapproval) suspends the turn
  and asks a human for real, and guardrails reject in code. These compose with
  soft approval, they do not replace it. The three soft-approval tools declare
  themselves read-only so the hard gate never stops the act of asking a human.
- **Not deferrable.** All three tools are `DeferrablePolicy::Never`: the model
  has to find `request_approval` at the instant it decides to pause, so tool
  search must not hide it.
- **No bespoke approval UI.** Consent is spoken in the conversation; there is
  deliberately no modal or button. Hosts that want to render the pause read
  `PendingApprovalStore`.
