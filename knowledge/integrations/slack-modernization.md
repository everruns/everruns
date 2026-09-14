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

**Tool narration reuses `AgUiToolVisibility`.** Mapping tool names directly to Slack
status text would leak internals into a user-facing surface. AG-UI already solved this
with a `None` / `Generic` / `Narrated` policy; Slack consumes the same policy rather
than growing a second one. What a public surface may reveal stays decided in one place.
This is the one decision not yet exercised — it belongs to the open status work below.

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
| EVE-975 | Agent status and thread title from turn and tool lifecycle |
| EVE-978 | Decide where suggested prompts come from, or decide not to have them |
| EVE-988 | Startup recovery re-registers deliveries for cancelled turns |

EVE-975 is the last piece of the agent surface proper: the pane streams, but shows no
status while a tool runs. EVE-988 is a defect found verifying the above — terminal-state
handling was fixed in the live path and not in the recovery path.

Deliberately not tracked, each needing its own design pass first: interactivity and
approval buttons, Slack tools for the agent (including fetching the files users attach,
which are currently reduced to a filename string), the Workflow Builder custom step, and
the OAuth install flow. The last one requires revisiting the per-app bot identity
decision, which is why it is not simply a ticket.

## Sources

Slack platform behavior was verified against
[Slack's AI app documentation](https://docs.slack.dev/ai/developing-ai-apps/) and the
[app manifest reference](https://docs.slack.dev/reference/app-manifest/) in September
2026. Slack documents some methods as compatibility bridges over newer equivalents, so
the platform is still moving; the adapter isolates those calls for that reason.
