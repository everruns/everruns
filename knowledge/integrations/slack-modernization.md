---
type: Investigation
title: "Slack Integration Modernization"
description: "Why the Slack channel was rebuilt onto Slack's agent surface, what was decided, and what is left."
tags:
  - everruns
  - integrations
  - slack
  - messaging
---

# Slack Integration Modernization

## Abstract

The Slack channel was built against the classic Events API: a bot that received a
message, ran a turn, and posted one plain-text reply in a thread. Slack has since
shipped a first-class agent surface — a split-view container, app threads,
token-by-token streaming, agent session status and titles, suggested prompts — plus
manifest fields that remove most of the manual setup the integration used to require.

This concept records the September 2026 gap analysis against that platform, the design
decisions it produced, and what remains. Most of the work has landed; the reasoning is
kept here because it is not recoverable from the code, and because the two open items
depend on it.

Implementation detail lives in
[`crates/server/specs/slack-integration.md`](../../crates/server/specs/slack-integration.md);
the channel abstraction it instantiates lives in
[Messaging Integrations](messaging-integrations.md).

## Design decisions

**The surface is chosen at runtime, not by configuration.** Enabling Slack's Agents
feature does not replace the channel bot; the same app still answers `@mentions` in a
channel and *additionally* gains an assistant container. Which surface an event belongs
to is knowable from the event itself. So configuration carries a single
`agent_surface_enabled` boolean that governs the manifest and event subscriptions, and
delivery style is selected per event. A third `reply_mode`, or a separate `slack_agent`
channel type, were both considered and rejected: they model as static configuration
something that is a property of the inbound event.

A consequence worth stating, because it is not obvious and was decided rather than
discovered: the pane forces `PerThread` routing while channel threads keep whatever the
config says. Rejecting `per_channel`/`per_user` at config time would be wrong, since the
same app legitimately serves channels where those strategies mean something.

**`report_progress_only` is scoped rather than retired.** It exists because silence
during a long turn was unacceptable, and native streaming removes that need in the
assistant pane. It survives as the channel-thread answer, because token-by-token
streaming into a shared channel is not wanted. Streaming is the pane answer. Neither
obsoletes the other, and no existing app needed migrating.

**Tool narration reuses `PublicToolVisibility`.** Mapping tool names directly to Slack
status text would leak internals into a user-facing surface. AG-UI already solved this
with a `None` / `Generic` / `Narrated` policy; Slack consumes the same policy rather
than growing a second one. What a public surface may reveal stays decided in one place.
Exercised by EVE-975: the policy moved out of `api/ag_ui.rs` into
`everruns_platform::exposure::public_tool_activity_text`, which both surfaces now
call. Slack channels grew the same `tool_visibility` / `generic_tool_text` knobs so the
pane is configured like any other public surface rather than hard-coded.

**Streaming makes the delivery dispatcher stateful and clocked.** The dispatcher was
notification-driven and stateless between events. Streaming requires accumulating deltas
and flushing on a cadence, which needs a timer, and per-output-message stream state that
must be closed on *every* terminal event including `turn.cancelled`. An unstopped stream
is a Slack message that spins forever, which is worse than the silence it replaced — so
correct terminal-state handling was treated as a prerequisite for streaming rather than
a parallel concern, and the work was sequenced that way.

**Streaming routes through `ChannelDeliveryAdapter`, via a capability probe.** Adapters
expose optional streaming support rather than being forced to implement it; those that
do not fall back to discrete delivery. This required migrating the existing direct Slack
calls onto the trait first, which retired a dead-code problem — the trait had an
implementation that no caller used — instead of entrenching it.

## What shipped

Eleven changes, September 2026, in dependency order. Together they take the integration
from a classic Events API bot to a Slack agent app.

| Issue | Change |
|---|---|
| EVE-966 | Terminal states reach Slack; `turn.cancelled` no longer leaks a registration |
| EVE-967 | Message-subtype allowlist, so `channel_join` no longer burns a turn |
| EVE-968 | `Retry-After` honoured on rate limits, with a ceiling |
| EVE-969 | Thread backfill pages, and says so when it truncates |
| EVE-970 | Manifest carries `event_subscriptions`; publish precedes Slack app creation |
| EVE-971 | Replies post as markdown blocks, with correlation metadata |
| EVE-972 | Delivery routes through `ChannelDeliveryAdapter` |
| EVE-973 | Agent surface: manifest feature, scopes, events, per-event surface detection |
| EVE-974 | Pane replies stream token-by-token |
| EVE-976 | The pane stop button cancels the running turn |
| EVE-977 | `ThreadContext` persists and carries Slack's reported context |

## What is left

| Issue | Item |
|---|---|
| — | Nothing outstanding. |

EVE-978 settled the last gap: suggested prompts come from the agent's conversation
starters, falling back to the harness's, resolved by
`everruns_platform::exposure::resolve_starters` and emitted into the manifest's
`features.agent_view.suggested_prompts`. The field was already there for Platform Chat,
so the prompts are authored by whoever configures the agent rather than generated, and an
agent with no starters emits no prompts at all — an empty pane beats prompts nobody wrote.
See "Suggested prompts come from conversation starters" in
[`crates/server/specs/slack-integration.md`](../../crates/server/specs/slack-integration.md).

EVE-975 and EVE-988 have since shipped. Two decisions from EVE-975 are worth keeping:

**Status is a capability, not a required method.** `ChannelAgentSurface` sits behind an
`agent_surface()` probe on `ChannelDeliveryAdapter`, the same shape streaming uses, so a
platform without a status line returns `None` instead of stubbing methods. It is pane-only
for the same reason streaming is — a channel thread has neither a status line nor a title.
Both calls are advisory: a failed status is logged and swallowed, because the reply is the
product and a decoration must never take a turn down with it.

**A pane rename writes back to the session title.** Titles already flow outward on
`session.title.updated`, so ignoring the inbound `agent_session_title_changed` would have
silently reverted a user's rename the next time the agent retitled the session — two
writers, one name. The write-back is scoped exactly like the stop button (a session the
receiving app owns, title and nothing else) and goes through
`session_title_updated_event`, whose no-op suppression is what stops the two directions
echoing each other.

Deliberately not tracked, each needing its own design pass first: interactivity and
approval buttons, Slack tools for the agent (including fetching the files users attach,
which are currently reduced to a filename string), and the Workflow Builder custom step.

**The OAuth install flow is no longer blocked** (EVE-1008). It was parked because it
required revisiting the per-app bot identity decision — one Slack workspace install maps
to one bot, and an App was a bundle of unrelated channels, so there was no single row to
install *into*. Retiring the App abstraction answered that: identity lives on the
endpoint. `signing_secret`, `bot_token` and `team_id` are endpoint config, each endpoint
serves its own manifest at `/v1/e/{endpoint_id}/slack/manifest` pointed at its own
request URL, verifies its own signatures, and is published on its own.

That gives the install flow an obvious shape: the callback writes the workspace's
credentials onto one endpoint row and publishes it, and two workspaces installing against
the same agent are two endpoints rather than a collision. What still needs deciding is
whether installing creates an endpoint or fills in one the operator made first, and where
the per-install team binding is enforced on inbound events. Neither is a modelling
question any more.

## Sources

Slack platform behavior was verified against
[Slack's AI app documentation](https://docs.slack.dev/ai/developing-ai-apps/) and the
[app manifest reference](https://docs.slack.dev/reference/app-manifest/) in September
2026. Slack documents some methods as compatibility bridges over newer equivalents, so
the platform is still moving; the adapter isolates those calls for that reason.
