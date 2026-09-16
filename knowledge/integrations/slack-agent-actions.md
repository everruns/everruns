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
created by a Slack channel carries `slack:app:{id}` tags; the capability walks
session → app → `SlackChannelConfig` → `bot_token` and acts as the bot the user
already invited. No second token, no second set of scopes to rotate, and the
agent's Slack identity matches the one answering in the thread.

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

1. Native `slack` capability with the channel's bot token.
2. Interactivity endpoint, manifest `settings.interactivity.request_url`, and
   the approval hint — the approval path end to end.
3. Task progress rendering in the delivery adapter.

## Files

- `crates/core/src/capabilities/` — capability registration
- `crates/core/src/tool_context.rs` — `session_store`, `session_task_registry`, the services a Slack capability needs
- `crates/server/src/api/slack_events.rs` — webhook, manifest, and the future interactivity endpoint
- `crates/server/src/slack_delivery.rs` — delivery adapter and progress rendering
- [Client Hints](../runtime-resources/client-hints.md) — the pause-and-consent mechanism this reuses
- [Slack Integration Modernization](slack-modernization.md) — where these three limits were recorded
