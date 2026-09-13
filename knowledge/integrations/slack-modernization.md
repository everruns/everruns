---
type: Investigation
title: "Slack Integration Modernization"
description: "Gap analysis of the Slack channel against the current Slack agent platform, with a prioritized set of improvements."
tags:
  - everruns
  - integrations
  - slack
  - messaging
---

# Slack Integration Modernization

## Abstract

The Slack channel was built against the classic Events API: a bot that receives a
message, runs a turn, and posts one plain-text reply in a thread. Slack has since
shipped a first-class agent surface (a dedicated split-view container, app threads,
token-by-token streaming, agent session status and titles, suggested prompts) plus
manifest fields, interactivity, and Workflow Builder steps that remove almost all of
the manual setup we still ask users to perform. This concept records what the
integration does today, where it falls behind that platform, and which gaps are worth
closing first. It is a gap analysis, not a commitment; each item still needs its own
design and issue.

Implementation details live in
[`crates/server/specs/slack-integration.md`](../../crates/server/specs/slack-integration.md);
the channel abstraction it instantiates lives in
[Messaging Integrations](messaging-integrations.md).

## Where the integration stands

Verified against `crates/server/src/api/slack_events.rs`,
`crates/server/src/slack_delivery.rs`, and
`apps/ui/src/components/apps/slack-setup-guidance.tsx`.

What works: per-app Slack apps with manifest generation, HMAC-SHA256 signing
verification with a replay window, `event_subscriptions`-driven inbound handling,
three session-routing strategies, DB-level deduplication of the `app_mention`/`message`
double delivery, thread-history backfill on first mention, Slack user identity resolved
into `ExternalActor`, image and file attachment parsing, event-driven outbound delivery
with retry and startup recovery, and a five-step setup checklist that self-advances from
observed webhook state.

What is absent: every Slack surface other than "a message arrives, a message is posted."
There is no interactivity endpoint, no slash command, no App Home, no link unfurl, no
Workflow Builder step, no agent/assistant surface, no streaming, no Block Kit, and no
OAuth install flow.

## Gaps

### 1. The agent surface is the headline gap

Slack now exposes agents as a dedicated product surface rather than as a bot user in a
channel: a split-view container, a top-navigation entry point, app threads, text
streaming, and suggested prompts. An app opts in by enabling the Agents feature, which
adds `assistant:write`, and by subscribing to `app_home_opened`, `message.im`,
`app_context_changed`, `agent_session_stopped`, and `agent_session_title_changed`.

Everruns is exactly the workload that surface exists for, and we render as a plain bot.
Concretely, five capabilities map onto runtime state we already have and currently throw
away:

| Slack capability | Everruns state that feeds it |
|---|---|
| `chat.startStream` / `chat.appendStream` / `chat.stopStream` | `output.message.delta` events, today only consumed by the UI |
| `agents.sessions.setStatus` | turn lifecycle and the currently running tool |
| `agents.sessions.rename` | the session title we already synthesize |
| `assistant.threads.setSuggestedPrompts` | agent card / harness affordances |
| `agent_session_stopped` | turn cancellation, which Slack users cannot currently trigger at all |

Streaming is the single largest perceived-quality change available. Today a Slack user
sees nothing between "message sent" and a finished reply that may be minutes away; the
`report_progress_only` handoff mode exists precisely because that silence was
unacceptable, and it works by asking the model to narrate, which is neither reliable nor
free. Native streaming makes the workaround unnecessary for the common case.

`app_context_changed` also carries what the user is currently looking at in Slack. That
is real context an agent could use, and we have no equivalent signal from any other
channel.

### 2. Setup is five manual steps that could be close to one

The current flow is: create the Slack app from a manifest, copy the signing secret,
copy the bot token, publish the Everruns app, then hand-configure Event Subscriptions
and re-subscribe to four bot events by hand. Steps four and five exist only because the
generated manifest omits `event_subscriptions`, on the stated grounds that it "requires
a live webhook URL."

That constraint does not hold. `settings.event_subscriptions.request_url` and
`settings.event_subscriptions.bot_events` are manifest fields, and the webhook URL is
fully determined by the app's public ID before the Slack app exists. The real
dependency is ordering: Slack verifies the URL when the manifest is saved, so the
Everruns app must be published first. Reordering publish ahead of Slack app creation
collapses steps four and five into the manifest, and the same manifest can carry
`settings.interactivity.request_url`, `features.slash_commands`, `features.app_home`,
and the agent feature toggle at no extra cost to the user.

Two credentials still have to be copied by hand. Removing that needs an OAuth install
flow (`oauth/v2/access` behind an "Add to Slack" button), which is a genuine design
decision rather than an oversight: it means one distributed Everruns Slack app across
workspaces, which trades away the per-app bot identity (name, avatar, scopes) that the
current design deliberately chose. A hybrid is possible — OAuth for the fast path,
manifest-per-app for teams that want their own bot identity — and should be evaluated
on its own rather than assumed.

Local development deserves its own note: the checklist tells users to run `ngrok`.
Socket Mode exists for exactly this and is explicitly disabled in our manifest
(`socket_mode_enabled: false`).

### 3. Failures are silent in Slack

`extract_delivery_text` returns text only for `output.message.completed` (or a
`report_progress` tool result in handoff mode). Every other terminal outcome produces
nothing at all in the Slack thread:

- `turn.failed` unregisters the delivery and logs a warning. The Slack user sees the
  message they sent and no reply, forever.
- Budget exhaustion and guardrail blocks reach Slack the same way: as silence.
- A turn that completes without a text output message is indistinguishable from a
  crash.
- `post_to_slack_with_retry` exhausting its three attempts, or failing on a permanent
  error such as `channel_not_found`, logs server-side and stops. Nothing tells the user
  the answer was produced and lost.

This is the most user-visible defect in the integration and the cheapest to fix. A
terminal-state handler that posts a short failure notice, ideally with a link back to
the Everruns session, would close it.

### 4. Output fidelity is plain text

Delivery posts `{channel, text, thread_ts}` and nothing else. Consequences:

- Agent output is Markdown; Slack's `text` field is `mrkdwn`. Tables, headings, nested
  lists, and fenced code with language hints degrade. Slack's `markdown` block accepts
  real Markdown and is the direct fix.
- Long answers are a wall of text in a thread. A Canvas would suit a long report
  better, and the Canvas API can create one from the thread.
- There is no way to attach a file the agent produced; `files.getUploadURLExternal` /
  `completeUploadExternal` is unused.
- `chat.postMessage` takes a `metadata` parameter. Stamping the Everruns session and
  message ID onto every posted message would give us a durable correlation key and a
  `message_metadata_posted` event, replacing tag-string heuristics.

### 5. Inbound is one shape only

Everything arrives as a channel or DM message. Not implemented, in rough order of value:

- **Interactivity** (`settings.interactivity.request_url`, `block_actions`, `views.open`).
  Buttons are the natural Slack expression of human-in-the-loop approval, and modals are
  the natural way to collect structured input. Without them an agent cannot ask a Slack
  user to confirm anything.
- **Workflow Builder custom steps** (manifest `functions`). This is the strongest
  "one-button" story available: an Everruns agent becomes a drop-in step any Slack admin
  can wire into a workflow with no code, driven entirely by manifest declaration.
- **Slash commands** (`features.slash_commands`, up to 50) for explicit invocation
  without an `@mention`.
- **App Home** (`features.app_home`) as a per-user landing surface: recent sessions,
  configuration, status.
- **Link unfurling** (`links:read`, `link_shared` → `chat.unfurl`) so an Everruns
  session URL pasted into Slack renders as a rich preview.

### 6. The agent cannot act on Slack

[Messaging Integrations](messaging-integrations.md) already records platform-contributed
tools as a known gap: `Capability::tools()` exists and no channel adapter uses it. The
consequences are concrete. An agent cannot acknowledge with a reaction, post to a
different channel, open a thread, upload a file, or set its own status. It also cannot
read the files a user attached: attachments become an `[Attached file: name (type)]`
string, and `url_private` is never fetched even though `files:read` is already in the
manifest scopes. A PDF dropped into the thread is invisible to the agent.

### 7. Scoping is coarse

`channel_id` is a single optional channel. There is no allowlist, no
DM-only mode, no per-workspace configuration under one app, and no Enterprise Grid
handling (`org_deploy_enabled: false` in the manifest). `token_rotation_enabled: false`
means bot tokens never rotate, which is a posture worth revisiting before any
marketplace distribution.

### 8. Implementation debt found while reading

Each of these is small and independently fixable:

- **`SlackDeliveryAdapter` is dead code.** It implements `ChannelDeliveryAdapter`, but
  `SlackDeliveryDispatcher` calls `post_to_slack_with_retry` directly and never goes
  through the trait. The abstraction the parity requirements are written against is not
  actually exercised by the only implementation, so a second platform will discover its
  gaps rather than inherit a proven path.
- **`ThreadContext` is constructed and discarded.** `process_slack_message` builds one
  per message, tracks the participant, logs, and drops it; nothing persists. The
  `participants_summary()` line never reaches the LLM. The `TODO` in the code
  acknowledges this. Session participants are tracked separately and durably, so the
  `ThreadContext` call is currently pure overhead.
- **Message subtypes are not filtered.** Only `bot_message` is skipped. A `channel_join`
  event is `type: message` with non-empty text, so inviting someone to the channel
  creates a session and burns a turn. An explicit subtype allowlist is needed.
- **Rate limiting ignores `Retry-After`.** `ratelimited` is retried on a fixed
  1s/2s/4s backoff. Slack tells us how long to wait; honoring the header would remove
  avoidable failures under load.
- **Thread backfill is capped and unpaginated.** `conversations.replies` is fetched with
  `limit=100` and no cursor follow-up, and only for a brand-new `per_thread` session.
  Longer threads silently lose their oldest context.
- **Bot messages are dropped unconditionally.** This prevents loops, but it also makes
  agent-to-agent and app-to-agent interaction in a Slack thread impossible. A bot-ID
  allowlist, or skipping only our own bot ID, would preserve loop safety while allowing
  it.

## Agent surface: design decisions

Settled in review of this analysis. Recorded here because they are not recoverable from
the code that will implement them.

**The surface is chosen at runtime, not by configuration.** Enabling Slack's Agents
feature does not replace the channel bot; the same app still answers `@mentions` in a
channel and *additionally* gains an assistant container. Which surface an event belongs
to is knowable from the event itself. So configuration carries a single
`agent_surface_enabled` boolean that governs the manifest and event subscriptions, and
delivery style is selected per event. A third `reply_mode`, or a separate
`slack_agent` channel type, were both considered and rejected: they model as static
configuration something that is a property of the inbound event.

**`report_progress_only` is scoped rather than retired.** It exists because silence
during a long turn was unacceptable, and native streaming removes that need in the
assistant pane. It survives as the channel-thread answer, because token-by-token
streaming into a shared channel is not wanted. Streaming is the pane answer. Neither
obsoletes the other, and no existing app needs migrating.

**Tool narration reuses `AgUiToolVisibility`.** Mapping tool names directly to Slack
status text would leak internals into a user-facing surface. AG-UI already solved this
with a `None` / `Generic` / `Narrated` policy; Slack consumes the same policy rather
than growing a second one. What a public surface may reveal stays decided in one place.

**Streaming makes the delivery dispatcher stateful and clocked.** `SlackDeliveryDispatcher`
is notification-driven and stateless between events. Streaming requires accumulating
deltas and flushing on a cadence, which needs a timer the dispatcher does not have, and
per-output-message stream state that must be closed on *every* terminal event including
`turn.cancelled`. An unstopped stream is a Slack message that spins forever, which is
worse than the silence it replaces — so correct terminal-state handling is a
prerequisite for streaming, not a parallel concern.

**Streaming routes through `ChannelDeliveryAdapter`, via a capability probe.** Adapters
expose optional streaming support rather than being forced to implement it; those that
do not fall back to discrete delivery. This also requires migrating the existing direct
Slack calls onto the trait first, which retires the dead-code problem recorded above
instead of entrenching it.

## Tracked work

Every item below is filed on the EVE team. This concept is the rationale; the issues are
the plan of record.

| Issue | Item |
|---|---|
| EVE-966 | Terminal states are silent in Slack, and `turn.cancelled` leaks a registration |
| EVE-967 | `channel_join` creates a session and burns a turn |
| EVE-968 | Rate-limit retry ignores `Retry-After` |
| EVE-969 | Thread backfill truncates at 100 messages |
| EVE-970 | Manifest carries `event_subscriptions`; publish before app creation |
| EVE-971 | Markdown blocks instead of raw `mrkdwn` |
| EVE-972 | Route delivery through `ChannelDeliveryAdapter` |
| EVE-973 | Enable the agent surface: manifest, scopes, events, surface detection |
| EVE-974 | Stream replies via `chat.startStream` |
| EVE-975 | Agent status and thread title from turn and tool lifecycle |
| EVE-976 | Stop a running turn via `agent_session_stopped` |
| EVE-977 | Persist `ThreadContext`; feed it `app_context_changed` |
| EVE-978 | Decide where suggested prompts come from |

Not filed, and deliberately so: interactivity and approval buttons, Slack tools for the
agent, the Workflow Builder custom step, and the OAuth install flow. Each needs its own
design pass first, and the last one requires revisiting the per-app bot identity
decision.

## Sources

Slack platform behavior cited above was verified against
[Slack's AI app documentation](https://docs.slack.dev/ai/developing-ai-apps/) and the
[app manifest reference](https://docs.slack.dev/reference/app-manifest/) in September
2026. Exact scope, event, and method names should be re-checked against those pages
before implementation; the platform is moving quickly and some methods are documented
as compatibility bridges over newer equivalents.
