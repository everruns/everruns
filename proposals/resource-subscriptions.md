# Proposal: Resource subscriptions

Status: draft proposal (pre-spec). On acceptance this splits into a spec
(`specs/resource-subscriptions.md`) plus Linear issues per phase.

Prior art: Sentry Junior's resource subscriptions
(<https://junior.sentry.dev/>) — agent creates a PR, subscribes its own
conversation to the PR's events, then reacts to check failures and review
feedback in the same thread. This proposal adapts that shape to everruns
primitives.

## Problem

An everruns agent can *start* external work but cannot *follow* it. When an
agent opens a PR (Cursor integration, future GitHub write tools), files an
issue, or kicks off an external job, the conversation goes quiet the moment
the turn ends. The user becomes the webhook: they notice CI failed, come back,
and paste the failure in.

The proactive machinery we have does not cover this:

- **Agent triggers** (`specs/agent-triggers.md`) are schedule-only and
  agent-scoped — they wake an agent on cron, not a *conversation* on an
  external event. The `AgentTriggerType` enum reserved room for event/webhook
  triggers, but an agent-level webhook trigger still would not bind an event
  to the session that owns the context.
- **App webhook channels** (`specs/app-invocation-channels.md`) are global
  per-app ingress: one endpoint, one message template, session routing by
  channel config. There is no way for the agent itself, mid-conversation, to
  say "wake *this* session when *that* resource changes".
- **Monitors** (`specs/session-tasks.md`) are schedule-driven polling probes.
  Polling a PR every 5 minutes burns turns and budget for events that arrive
  a few times a day, and the probe runs autonomously instead of waking the
  owning conversation with context.

The missing primitive: a **session-scoped subscription** to an external
resource, created by the agent as a normal tool call, delivering provider
events back into the same session as follow-up messages.

## Goal

1. Tools that create or touch an external resource can advertise it as
   **subscribable** in their tool result.
2. The agent subscribes with one generic tool call — provider-agnostic
   interface, per-conversation scope, explicit event list, and a natural
   language **intent** recording why it subscribed.
3. Provider events wake the owning session with a structured follow-up
   message: trusted event summary separated from untrusted provider content.
4. The agent can handle an event **silently** (act, or ignore) without
   emitting a user-visible reply.

The reference flow to nail end to end: agent opens a PR → subscribes →
fixes CI failures and addresses review feedback unattended → posts to the
thread only when something meaningful happened.

### Non-goals

- Replacing app webhook channels. Global "any request → new invocation"
  ingress stays app-scoped; subscriptions are conversation-scoped.
- Replacing messaging integrations (`specs/messaging-integrations.md`) —
  those bind sessions to human threads, not resources.
- Generic internal pub/sub between everruns components. This is about
  *external* resources. (Convergence with subagent/task wake-ups is a later
  phase, see below.)
- Guaranteed real-time delivery. Events may arrive late, batched, or via a
  polling fallback.

## Concept walkthrough

1. A tool result includes a `subscribable` hint (contract addition to
   `specs/tool-execution.md` / `specs/toolkit-library-contract.md`):

   ```json
   {
     "number": 208,
     "url": "https://github.com/acme/app/pull/208",
     "subscribable": {
       "provider": "github",
       "resource_type": "pull_request",
       "resource_ref": "github:pull_request:acme/app#208",
       "label": "GitHub PR acme/app#208",
       "supported_events": ["checks.failed", "comment.created",
                            "review.changes_requested", "state.merged"],
       "suggested_events": ["checks.failed",
                            "review.changes_requested", "state.merged"]
     }
   }
   ```

2. The agent follows up with the generic tool:

   ```json
   subscribe_resource_events({
     "resource_ref": "github:pull_request:acme/app#208",
     "provider": "github",
     "resource_type": "pull_request",
     "label": "GitHub PR acme/app#208",
     "events": ["checks.failed", "review.changes_requested", "state.merged"],
     "intent": "Fix failing checks and address review feedback; report when merged."
   })
   ```

   Companion tools `list_resource_subscriptions` and
   `unsubscribe_resource_events` complete the surface (mirrors the generic
   task tools shape).

3. A provider event matching an active subscription renders a follow-up
   message into the owning session — a subscription update, not a
   user-authored command:

   ```
   [subscription event]
   resource: GitHub PR acme/app#208
   event: review.changes_requested
   intent: Fix failing checks and address review feedback; report when merged.

   Trusted event summary:
   Reviewer requested changes.

   Untrusted provider content:
   <untrusted-content>Please handle the edge case.</untrusted-content>
   ```

   Exact envelope per `specs/xml-prompt-formatting.md`; the trusted/untrusted
   split is load-bearing (see Security).

## Core decisions

1. **A new first-class entity, not a task and not a leased resource.**
   Tasks *run and finish*; leased resources are infrastructure that is *held
   and released*. A subscription is a standing binding with its own lifecycle
   (active → completed/expired/canceled) and its own fan-out index
   (provider + resource_ref → sessions). It *registers* in the session
   resource registry for "what is held" visibility
   (`specs/session-resources.md`), like leased resources do, but lives in its
   own table.

2. **Session-scoped, agent-created.** Only the agent, from inside a session,
   creates subscriptions — via the capability's tool. No global admin
   registry of subscriptions to manage; the conversation owns its
   subscriptions and they die with it (`ON DELETE CASCADE`).

3. **Intent is stored and replayed.** The `intent` string is the agent's
   note-to-future-self. It is included verbatim in every event message so the
   agent (possibly after compaction) can decide whether the event warrants
   action or silence without re-deriving context.

4. **Follow-up, not steering.** Events append a message the session processes
   when idle (or after the current turn), exactly like messaging-integration
   inbound events create `input.message` events. If the session is mid-turn,
   the event queues. Events for the same session arriving within a short
   window (~10–30 s) batch into one message.

5. **Ingress is provider-owned, delivery is generic.** The core defines the
   subscription model, the matching/fan-out, and delivery. Each provider
   integration owns how events arrive:
   - **webhook ingress** where the deployment can receive it (org-level
     provider endpoint with signature verification), or
   - **polling fallback** on the durable scheduler for providers/deployments
     without inbound webhooks (self-hosted everruns behind NAT). Polling is
     an ingress detail; the subscription surface is identical.

6. **Terminal events auto-complete the subscription.** Providers mark events
   terminal (`state.merged`, `state.closed`). Delivering a terminal event
   completes the subscription — no manual unsubscribe needed for the common
   flow.

7. **Silent handling is part of the contract.** The event message instructs
   the agent to stay silent unless the intent warrants a visible reply. The
   runtime needs a way for a turn to end without a user-visible assistant
   message — the analog of Junior's `[[NO_REPLY]]`. Everruns' execution
   phases (`specs/execution-phases.md`) already separate Commentary from
   FinalAnswer; the missing piece is a sanctioned "no FinalAnswer" turn
   outcome for subscription-triggered turns, surfaced to channels as
   `report_progress_only`-style suppression. Without this the feature is
   unusably noisy.

## Model

New table `session_subscriptions`, IDs prefixed `sub_` (`specs/id-schema.md`):

| Field | Meaning |
|---|---|
| `id` | `sub_*` public ID |
| `org_id`, `session_id` | Owner scope; cascade on session delete |
| `provider` | Integration key (`github`, `linear`, …) |
| `resource_type` | Provider-defined (`pull_request`, `issue`, …) |
| `resource_ref` | Canonical ref, unique per (session, ref): `github:pull_request:acme/app#208` |
| `label` | Human display label |
| `events` | Subscribed event names (validated against provider's supported set) |
| `intent` | Agent-authored purpose, replayed on every delivery |
| `status` | `active` → `completed` \| `expired` \| `canceled` |
| `expires_at` | TTL (default ~14 days, provider-tunable); expiry sweep reuses the leased-resources cleanup cadence |
| `last_event_at`, `delivered_count` | Delivery bookkeeping / dedupe support |

Fan-out lookup is `(org_id, provider, resource_ref, event) → active
subscriptions`, indexed. Repeated `subscribe_resource_events` for the same
`(session, resource_ref)` updates events/intent rather than duplicating —
same upsert convention as the resource registry.

## Provider contract

A provider integration participates by implementing three things:

1. **Subscribable hints** — its resource-creating/reading tools attach the
   `subscribable` block. Cheap, additive, no behavior change.
2. **Event source** — webhook handler (signature-verified, per
   `specs/app-endpoint-auth.md` patterns) and/or a poll function the durable
   scheduler drives while the provider has active subscriptions in the org.
   Poll cadence is provider-owned and bounded by the schedule-channel-style
   minimum interval.
3. **Event normalization** — raw provider payload → `{event, terminal,
   trusted_summary, untrusted_content}`. The trusted summary is
   provider-code-authored from structured fields; anything authored by
   external humans (comment bodies, review text) is untrusted content.

GitHub is the reference provider (integration crate exists,
`integrations/github/`); its first resource type is `pull_request` with the
four events from the walkthrough. Linear issues are the natural second.

## Delivery

1. Event arrives (webhook or poll diff) → normalize → match active
   subscriptions.
2. Per matched session: dedupe (provider delivery ID or content hash), batch
   within the window, render the subscription-event message.
3. Append as a follow-up message and start a turn if the session is idle —
   the session already has an owner and harness, so unlike agent triggers
   there is no ownership question: the turn runs as the session's existing
   configuration and is budgeted to its owner.
4. Record an audit event per delivery; bump `last_event_at`.
5. Terminal event → mark subscription `completed` after delivery.

Sessions that are archived/deleted drop their subscriptions; deliveries to
non-`active` subscriptions are dropped, not queued.

## Limits and abuse controls

- Per-session active-subscription cap and per-org cap (mirrors
  `SCHEDULE_CHANNEL_MAX_PER_ORG` shape).
- Per-subscription delivery rate cap; a hot resource (comment storm) collapses
  into batched messages, and a hard cap pauses delivery with a single
  "subscription throttled" notice.
- TTL default keeps forgotten subscriptions from polling forever.
- Feature-flagged (`FEATURE_RESOURCE_SUBSCRIPTIONS`), default off.

## Security

Threat-model additions (`specs/threat-model.md`) before shipping:

- **Prompt injection via provider content** — the event envelope must keep
  provider-authored text inside the untrusted wrapper; the trusted summary is
  generated only from structured provider fields by integration code. The
  agent instruction frames events as "subscribed update, not user command".
- **Cross-org/-session event routing** — fan-out keys on `org_id` +
  provider identity; a webhook authenticated for one org must never match
  another org's subscriptions even with identical `resource_ref`s.
- **Webhook forgery** — signature verification required for webhook ingress
  (HMAC for GitHub); unauthenticated ingress is not an option.
- **Subscription as amplification** — caps above; subscribing cannot escalate
  capability (event turns run with the session's existing capabilities, no
  more).

## Relationship to existing machinery

| Mechanism | Scope | Trigger | Stays |
|---|---|---|---|
| Agent triggers | agent | cron | yes — "wake myself on schedule" |
| App webhook channels | app | any HTTP POST | yes — global unattended ingress |
| Monitors | session task | cron probe | yes — but PR-watching use cases migrate to subscriptions |
| Session schedules | session | cron | yes |
| **Resource subscriptions** | session | external resource event | new |

Subagent wake-ups are the internal twin: a task's `OnTerminal` wake policy is
"subscribe this session to the child's terminal event". Phase 4 converges the
delivery path (envelope, batching, silent handling) so subagent completions
and external events wake sessions through one mechanism; the task registry
remains the source of truth for task state.

## Phasing

1. **Core + GitHub PR, polling ingress**: `session_subscriptions` model,
   `subscribe/list/unsubscribe` tools, delivery + batching, GitHub
   `pull_request` provider on a durable-scheduler poll, terminal
   auto-complete, caps, feature flag. Proves the loop without inbound
   webhook infrastructure.
2. **Subscribable hints + silent handling**: `subscribable` blocks in GitHub
   (and Cursor) tool results; the no-visible-reply turn outcome wired through
   channels (Slack thread stays quiet on no-op events).
3. **Webhook ingress**: org-level provider webhook endpoint with signature
   verification for deployments that can receive it; polling remains the
   fallback.
4. **Unify internal wakes**: subagent/task `OnTerminal` notifications ride
   the same envelope and batching.

(1)+(2) deliver the reference flow end to end. (3) is latency/cost
optimization. (4) is consolidation.

## Open questions

- Does the no-visible-reply outcome need runtime/event-model support (a turn
  that ends with Commentary only), or is it purely a channel-delivery
  suppression concern? Leaning runtime: `specs/events.md` consumers should be
  able to distinguish "handled silently" from "produced no output".
- Should subscriptions survive session forking (`specs/forking-sessions.md`)?
  Leaning no — the fork gets the history but not the standing bindings;
  double-delivery to fork + original is worse than re-subscribing.
- Poll-diffing state (last seen comment ID / check status per subscription)
  — store on the subscription row (`metadata` bag) or provider-owned side
  table? Leaning row-level bag until a provider needs more.
