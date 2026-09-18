---
type: Decision
title: "Slack Agent Actions"
description: "Why approvals, task progress, and the second-identity problem are one missing capability, and how the existing pause-and-consent machinery supplies the approval half."
tags:
  - everruns
  - integrations
  - slack
  - messaging
  - capabilities
---

# Slack Agent Actions

## Abstract

Three limits were recorded against the Slack integration after the agent surface
landed: an agent cannot ask for approval with a button, task progress is not
rendered into Slack, and reaching the Slack API at all requires attaching an MCP
server with a second bot token.

They are one problem. **An agent cannot act on its own Slack channel using that
channel's identity.** Everything else follows from that, and the approval half
needs no new protocol — Everruns already has a pause-and-consent mechanism used
twice, and Slack is simply a third client for it.

This concept records that framing and the resulting shape. It supersedes the
"Future: Platform Tools" note in
[Messaging Integrations](messaging-integrations.md), which recorded the gap
without a design.

## The three limits, restated

| Recorded as | Actually |
|---|---|
| No approval buttons | Slack cannot render a pause-and-consent card, and no endpoint receives the click |
| Progress is per-turn, not per-task | Task state is already reachable; nothing renders or updates it in Slack |
| Two identities | No native capability resolves the channel's own bot token, so MCP is the only route |

## Decisions

**A native `slack` capability, resolving the channel's bot token.** A session
created by a Slack channel carries both `slack:app:{id}` and
`slack:endpoint:{id}` tags. The capability resolves through the **endpoint**,
not the app: since EVE-1008 the endpoint is what owns Slack bot identity, so one
agent can carry two Slack endpoints with different bots and resolving via the
app would pick the wrong one. It then acts as the bot the user already invited —
no second token, no second set of scopes to rotate, and the agent's Slack
identity matches the one answering in the thread.

This is the first real use of `Capability::tools()` by a channel adapter, which
the parity requirements anticipated and no adapter had exercised.

Rejected: keeping the Slack MCP server as the supported answer. It works, and it
should keep working for anything exotic, but it makes the common cases
(acknowledge with a reaction, update a status message) cost a credential the
operator must provision, scope, and rotate separately from the one they already
gave us.

**Approvals reuse the existing pause-and-consent machinery, and do not invent a
protocol.** `setup_connection` and `url_elicitation`
([Client Hints](../runtime-resources/client-hints.md)) already establish the
shape: a client declares a hint, a tool that needs a human emits a synthetic
tool call, the session moves to a waiting state, the client renders the
interaction and posts the decision back, and the turn resumes.

Slack becomes a third client of that pattern. The Slack channel declares the
hint when it creates a session; the delivery adapter renders the request as Block
Kit; the click arrives at a new interactivity endpoint and posts the decision.

The degradation path is already designed and is exactly today's behaviour: a
channel that does not declare the hint gets no pause, the tool result carries the
reason, and the model asks in prose. The client-hints concept states the reason
to keep it that way — "pausing a turn on a card nobody draws just burns the
tool-result timeout."

**Progress is rendered by the delivery adapter, not narrated by the model.**
`session_task_registry` is already on `ToolContext`, so task state is reachable
today; the gap is that nothing turns it into a Slack message. Having the model
narrate its own task list is unreliable and costs tokens on every turn. The
adapter posts one status message and `chat.update`s it as tasks settle, using
the correlation metadata already stamped on posted messages.

## Who may approve

The interactivity endpoint is unauthenticated in the same sense the events
endpoint is: Slack signs it, nothing else does. That makes authorization a
design decision rather than a detail.

A click must be bound to **both** the specific pending tool call and an
identified Slack user. Binding only to the tool call means anyone who can see
the channel can approve consequential work — the "capability follows the channel"
posture that external analyses of comparable products have flagged as the weak
point. The approving user's identity is recorded on the decision regardless of
policy, so an approval is always attributable after the fact.

Whether approval is restricted to the requester, to an allowlist, or open to any
channel member is per-channel configuration. The default should be the
requester, because that is the only choice that is never surprising.

## Shape

```
Agent needs a decision
  └─ request_approval tool → synthetic tool call, session waits
       └─ Slack delivery adapter renders Block Kit buttons in the thread
            └─ human clicks
                 └─ POST /v1/apps/{app_id}/slack/interactivity   (Slack-signed)
                      └─ resolve to the pending call + the clicking user
                           └─ authorize, record who decided
                                └─ post the decision → turn resumes
```

The endpoint mirrors the events endpoint: same signing secret, same unscoped app
lookup, same generic 404 for anything not published.

## Sequencing

The capability is the foundation; both other items need it to reach Slack as the
bot.

1. Native `slack` capability with the channel's bot token. **Landed (EVE-1024).**
2. Interactivity endpoint, manifest `settings.interactivity.request_url`, and
   the approval hint — the approval path end to end. **Landed (EVE-1025).**
3. Task progress rendering in the delivery adapter.

## What landed for step 1

One decision the abstract above did not anticipate: **the action crosses the
process boundary, not the token.**

The agent loop runs in the worker; `bot_token` lives in the endpoint row the
control plane owns. Handing the worker the token would put a long-lived
workspace credential in the process that also evaluates model-chosen tool
arguments, and would need a second Slack HTTP path beside the one
`slack_delivery` maintains. So `SlackActionInvoker` is a seam: the capability
names an action, the control plane resolves the endpoint and performs the call,
and only the outcome comes back. The token never leaves the control plane in
either deployment — in-process it is read directly, remote it is used behind the
`InvokeSlackAction` RPC.

An invoker is **bound to one org and one session at construction**, the way
`platform_store(org_id, session_id)` is. A capability holds only the handle it
was given, so there is no argument it could vary to reach another session's
endpoint. Org scoping is then structural rather than a check: the session read
is `get_session(org_id, session_id)` and the app read is
`get_by_internal_id(.., org_id, ..)`.

Resolution prefers `sessions.endpoint_id` (EVE-1004) over the
`slack:endpoint:{id}` routing tag, because the FK is immutable and the tag is
not; the tag remains the fallback for pre-backfill sessions. Either way the
endpoint must be `ChannelType::Slack` and `status == live`, so a session that
came through another channel never falls through to a sibling Slack endpoint —
the wrong-bot bug that resolving by endpoint exists to prevent.

Errors cross as a closed enum, not a message string. The capability renders "this
session did not come from Slack" as a tool error the model should act on and a
transient fault as an internal error, and telling those apart must not depend on
parsing prose. An unrecognised kind — a control plane newer than the worker —
reads as transient, so a worker never tells a model something false about its
session.

`post_to_channel` is not shipped. It widens blast radius from "the thread that
asked" to "anywhere the bot is", and the reply path already answers in the
thread; it returns when there is a per-channel allowlist to gate it.

### Degradation

An adapter with no route to the control plane provides no invoker, and the tools
fail closed with the same reason a non-Slack session gets. That is the designed
answer rather than a gap: a capability that cannot resolve an endpoint must not
act as any other endpoint's bot.

## What landed for step 2

The abstract above assumed approvals would reuse the *synthetic tool call*
variant of pause-and-consent that `setup_connection` and `url_elicitation` use.
They do not need to, because `soft_approval` does not work that way.

`request_approval` is a real tool the model calls, and the model **ends its turn
on it**. The pause is answered by the user's next message, not by a client
posting a decision into a waiting turn. There is no turn to hold open, so there
is no synthetic call to emit and no `waiting_for_*` state to enter.

That makes the Slack half smaller than the shape above implies, and better:

- The delivery adapter turns a waiting `request_approval` into Block Kit buttons
  when the session declares the `slack_approval` hint.
- A click posts an ordinary user message on that session. The turn resumes
  through the path that already exists.

Answering as a message is also what keeps attribution honest for free.
`ApprovalAuditListener` resolves the approver from the `input.message` event the
API wrote — never from anything the model said — so a Slack click lands in the
audit log under the Slack identity that clicked it, and approvals need no
identity path of their own.

### Authorization

The binding rides in the button's `value`: session, requester, turn, and the
action as it was worded on the card. Slack echoes it back unchanged and covers
it with the same signature as the rest of the request, so a viewer cannot edit
another person's button and a forged POST does not verify. The requester is
stamped at render time, so the click-time check is an equality test rather than
a lookup that could race the session.

Three refusals, all fail-closed:

- a clicker who is not the requester (the default policy; the refusal is
  ephemeral and visible, and nothing is recorded),
- a card whose binding names a session in another org, or a session belonging to
  a *different Slack endpoint on the same app* — one agent can carry two, and a
  click on one must not answer a pause raised through the other,
- a card with no requester to bind to, which nobody may answer rather than
  everybody.

### Double clicks and stale cards

Answering rewrites the card in place and drops its buttons, so a second click
has nothing to hit. That is bookkeeping avoided rather than bookkeeping added.

### Degradation, again

A session without the hint gets no card: the ask is posted as prose and answered
in prose, which is exactly today's behaviour. So is a card whose binding will not
fit in Slack's 2000-byte action value — a card whose click could not be
authorized is not drawn at all.

## Files

- `crates/core/src/capabilities/` — capability registration
- `crates/core/src/tool_context.rs` — `session_store`, `session_task_registry`, the services a Slack capability needs
- `crates/server/src/api/slack_events.rs` — webhook, manifest, and the future interactivity endpoint
- `crates/server/src/slack_delivery.rs` — delivery adapter and progress rendering
- [Client Hints](../runtime-resources/client-hints.md) — the pause-and-consent mechanism this reuses
- [Slack Integration Modernization](slack-modernization.md) — where these three limits were recorded
