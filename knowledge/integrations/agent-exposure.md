---
type: Decision
title: "Agent Exposure (retiring the App abstraction)"
description: "Make Agent the addressable entity by re-homing App channels as agent-owned Channels and folding invocation into Triggers, retiring App."
tags:
  - everruns
  - integrations
  - agents
  - apps
---
# Agent Exposure (retiring the App abstraction)

> Status: **Accepted and implemented.** The phases were tracked as EVE-998 through
> EVE-1011. App data is frozen for compatibility and historical attribution. Agent-owned
> channels are the management and ingress source of truth. The concept shipped as
> "Endpoints" and was renamed to **Channels**, see
> [Naming: Endpoint became Channel](#naming-endpoint-became-channel).
> [apps.md](apps.md) is the authoritative description of the retained compatibility
> contract.
>
> One decision beyond the original proposal: **App data is frozen rather than deleted.**
> Historical rows, foreign keys, budget subject values, and route aliases stay stable
> while all management and traffic-serving ownership moves to the Agent domain.

## Abstract

An [App](apps.md) is a deployment wrapper that binds a Harness and an Agent to channels.
Since agents became harness-owning and schedule invocation moved to
[Agent Triggers](../runtime-resources/agent-triggers.md), the wrapper adds a required
`harness_id` that duplicates the agent's, a nullable `agent_id` kept only to grandfather
pre-agent rows, and a publish switch whose blast radius is wider than any single
exposure. What it genuinely contributes — per-exposure auth, per-exposure identity,
per-exposure lifecycle, and a session-ownership anchor — is contributed by its
`app_channels` rows, not by the `apps` row.

This proposes making **Agent** the addressable entity and re-homing what App owns onto
two collections beneath it:

- **Channels** — a door. Something outside connects, and waits for an answer on the
  same transport. Slack, AG-UI, Public Chat, A2A, FCP, API endpoint.
- **Triggers** — an alarm clock. Something fires, a message is injected, nobody is
  listening. Schedule (exists today), webhook (moves here from channels).

`apps` is then frozen. No ingress URL changes for anyone already installed, because
channel identity moves *off* the owning entity and onto the channel itself.

## Why an Agent can absorb this now

Four App responsibilities have to land somewhere. Each already has a natural owner one
level down:

| App owns today | Where it goes | Why it survives the move |
|---|---|---|
| `harness_id` | deleted | The agent already pins one; App's copy is a second source of truth that the create form fills *from* the agent. |
| `agent_id`, version policy | agent is the parent; policy moves to the channel | A staging channel on `latest` and a prod channel `pinned` is a real case the App forces into two Apps. |
| `agent_identity_id` | channel, defaulting to the agent's | Two App channels needing different identities requires two Apps today. Per-channel is strictly more expressive. |
| `owner_principal_id` | **channel, mandatory** | Load-bearing for security. See [Invariants that must not move](#invariants-that-must-not-move). |
| `status: draft/published` | per-channel status + one agent-level suspend | A "published" App with three draft channels is a lie the 2-D `App.status × channel.enabled` matrix tells today. |

The three structural objections to collapsing App into Agent — N exposures per agent,
`agent_versions` immutability, and the App-as-security-boundary rule in
[app-api-keys.md](app-api-keys.md) — are objections to losing the **per-exposure row**,
not to losing the App. Every one is satisfied by an `agent_channels` row: the row is the
boundary an API key scopes to, the row is what publish toggles, and the row is what stays
out of the agent's config snapshot.

## Model

### Channel

One row per way in. Transport-typed config, exactly as `app_channels` is today — the
config stays a per-transport typed union (`SlackChannelConfig` and friends in
`crates/platform/src/app.rs`, moving to a platform channel module), because
`agent_surface_enabled` has no meaning for A2A and pretending otherwise produces a
lowest-common-denominator config that fits nothing.

What is *not* per-transport, and becomes a first-class column rather than a field inside
each config variant:

- `status`: `draft | live | disabled`
- `session_binding` (see [Session binding](#session-binding))
- `agent_identity_id`, nullable, defaults to the agent's lazy identity
- `agent_version_policy` + `agent_version_id`
- `owner_principal_id`, `resolved_owner_user_id`
- `auth`, the shared inline config from [channel-auth.md](channel-auth.md)

Everything above is currently either an App column (wrong grain — shared across channels
that want different values) or duplicated inside several `channel_config` variants (wrong
place — the shared exposure policy is not transport detail). The exposure policy that
Slack and AG-UI already share, `public_tool_activity_text`, is the proof: it lives in
`platform::app` today and is called from both `slack_delivery.rs` and `api/ag_ui.rs`. It
belongs in a transport-neutral `everruns_platform::exposure` module, and this design
forces that move rather than inventing it (EVE-1001, independent of every phase).

### Trigger

[Agent Triggers](../runtime-resources/agent-triggers.md) already exist with the right
shape: `trigger_type`, typed `config`, `session_mode`, `enabled`, a durable-schedule
binding, and `AgentTriggerType` explicitly left open for event/webhook kinds. Webhook
moves here and needs no new concept: an
[invocation channel](app-invocation-channels.md) is already defined as "inject a
configured user message into a session when an external trigger fires", with a message
template and a session mode and no reply path. That is a trigger that fires on an HTTP
hit instead of a cron tick.

### Why split Channels and Triggers at all

The question is not whether to split, it is **where to cut**. Other platforms cut
`channels | automations`, which keeps webhook on the channel side because it arrives over
HTTP. Cut on *transport* and the two groups have overlapping, half-empty field sets —
which is exactly the state `app_channels.channel_config` is in.

Cut instead on **is there a peer waiting for a reply**, and the field sets become
disjoint:

| | Channel | Trigger |
|---|---|---|
| Who starts it | external caller | a clock, or an event |
| Reply path | yes — `ChannelDeliveryAdapter`, optional streaming | none |
| Needs | auth policy, rate limit, tool visibility, reply mode, thread context | message template, session mode |
| Lifecycle verb | *publish* | *enable* |
| Session key | thread / conversation / requester identity | the trigger, or nothing |

Two tables, disjoint columns, no JSONB union hiding the difference. **One UI tab**, so the
split costs the user nothing — they read one list of "how this agent is reachable".

Keeping them as one table was considered: it buys a uniform enable/disable mechanic and
one budget subject type. It is rejected because the uniformity is cosmetic — a single
`enabled` flag over two lifecycles (`publish` gates a door, `enable` arms a clock) is what
produced today's `App.status × channel.enabled` matrix, and one budget subject can be
achieved with two subject types just as well.

## Session binding

Two enums used to do overlapping work: `SessionStrategy` (`per_thread`, `per_channel`,
`per_user`) for messaging channels, and `InvocationSessionMode` (`shared_session`,
`session_per_invocation`) for webhook and schedule. They answered the same question — *what
identity keys the session* — and having two of them is why "some channels are
session-per-event and some are one single session" read as an inconsistency rather than a
setting.

Unified into one `SessionBinding` in `crates/core/src/channel.rs` (EVE-1005), where every
existing value maps 1:1 and nothing is lost:

| `SessionBinding` | Key | Replaces |
|---|---|---|
| `Thread` | thread ref | `per_thread` |
| `Conversation` | channel/room id | `per_channel` |
| `Requester` | external actor id | `per_user` |
| `Endpoint` | the channel row (variant name predates the rename) | `shared_session` |
| `Ephemeral` | none, fresh session | `session_per_invocation` |

**The wire vocabulary deliberately did not move.** Each variant renamed in Rust but still
serializes to its legacy string, with the new name accepted as a read alias. That is what
made the unification migration-free: no `channel_config` JSONB was rewritten, and the API
and UI kept exchanging the values they already did. Renaming the wire values is a separate
change that has to carry a migration and a UI update; the type-level split was the part
that cost something, and it is gone. The routing **tag** segments (`thread`, `channel`,
`user`) are load-bearing in the same way and are pinned by test — a renamed segment
silently orphans every live session keyed under it.

Two properties follow, and both are already decided behavior rather than new invention:

**The transport constrains which bindings are offerable.** A trigger can only key on
`Endpoint` or `Ephemeral` — there is no thread to key on. Express that as a method on the
transport descriptor (`allowed_bindings()`), not as a second enum.

**The declared binding is a default the transport may override per event.** The Slack
assistant pane forces `PerThread` while channel threads keep the configured strategy, and
[Slack modernization](slack-modernization.md) records the reasoning: the surface is a
property of the inbound event, not of configuration, so rejecting `per_channel` at write
time would be wrong. Generalize that into `resolve_binding(declared, event)` and the pane
stops being a special case.

Routing tags keep their existing generated shape from `build_session_routing_tag()`,
`{transport}:{binding}:{key}`, with `agent:{agent_id}` and `endpoint:{channel_id}`
replacing `app:` and `app_channel:`.

## Publish and expose

Publish moves **to the channel**, with one agent-level override:

```
live(channel) = channel.status == live
              && agent.status == active
              && !agent.exposures_suspended
```

- Per-channel `status` is the everyday control: publish the Slack channel without
  flipping on the public chat channel sitting next to it.
- A new channel always starts `draft`, even when its App is already published. Creating
  a new door never opens it without a separate publish action.
- `agent.exposures_suspended` is the incident control — one switch, take the agent off the
  internet — which is what App unpublish is actually reached for. It leaves per-channel
  status untouched, so clearing it restores exactly the previously live set.
- `agent.status != active` must imply no live channel. Enforce at resolution time, not by
  writing rows, the same way the harness chain is folded behind the platform seam.
- Agent exposure state for lists and badges is **derived** (`any channel live`), never
  stored. A stored flag is a second writer.

This is a security improvement, not just a tidier lifecycle. Today, obtaining a Slack
manifest requires publishing the whole App (EVE-970 sequenced publish before Slack app
creation), which simultaneously exposes every other channel on it. Per-channel publish
narrows that gate to the channel that needs it.

## Channel identity and URLs

The hard constraint: ingress paths are embedded in Slack app manifests, A2A agent cards,
integrator code, and public chat links. Re-pointing them at `{agent_id}` breaks live
installs.

It does not have to change, because the `app_id` segment in those paths is already
redundant — the channel id is globally unique and already appears in half of them
(`crates/server/src/api/`). Mount ingress on the channel alone:

```
POST /v1/e/{channel_id}/slack/events
POST /v1/e/{channel_id}/ag-ui
POST /v1/e/{channel_id}/a2a
GET  /v1/e/{channel_id}/.well-known/agent-card.json
POST /v1/e/{channel_id}/sessions
GET  /c/{channel_slug}
```

Old `/v1/apps/{app_id}/…` paths stay mounted permanently as aliases that resolve from
`agent_channels.legacy_app_public_id`. They never read the frozen `apps` table or the
`app_channels` compatibility view. Nothing installed breaks, ever. Deprecate in docs,
not in code.

Worth doing on its own merits: `/v1/apps/{app_id}/ag-ui` carries no channel segment, so an
App cannot expose two URL-distinguishable AG-UI channels. Channel-scoped routes fix that
uniformly instead of per transport.

## Invariants that must not move

These are load-bearing for security and will break quietly if the migration treats them
as bookkeeping.

**Channel-owned session ownership.** App-channel sessions adopt
`app.owner_principal_id`, not the caller's — internal callers default to the system
principal, and `shared_session` reuse keys on the owner, so without the override reuse
never matches and ownership is unaccountable. `owner_principal_id` must live on the
channel row, not be collapsed to the agent. See
[app-invocation-channels.md](app-invocation-channels.md) and TM-AUTHZ-009, TM-A2A-007 in
[threat-model.md](../security/threat-model.md).

**Reserved tag prefixes are append-only.** `__internal:`, `app:`, `app_channel:`,
`slack:app:`, and `ag_ui:app:` are rejected from non-internal callers at session create and
update (`crates/server/src/domains/sessions/service.rs`). New prefixes (`endpoint:`, and
any `agent:`-scoped routing tag) must be added to the reserved list **before** anything
writes them, and the old prefixes must stay reserved forever — a member who can seed a
session carrying a retired prefix can be adopted by an old-shaped lookup.

**Tag containment, not equality.** Reuse lookups require the candidate's tags to contain
every routing tag (`tags @> $tags`) and are keyed on org + owner, which is what stops
cross-org and cross-app adoption. Re-homing the tags must preserve both the containment
semantics and the org/owner keys.

## Slack, on the new integration

The Slack channel is now an agent app, not an Events API bot
([slack-modernization.md](slack-modernization.md)). Three consequences for this design,
two of which are arguments *for* it:

**Per-channel bot identity is the shape the parked OAuth work needs** (EVE-1008,
landed). The install flow was blocked on revisiting the per-App bot identity decision.
One workspace install maps to one channel, never to a bundle of App channels — the App was
never the natural owner of a Slack install. That decision now has the right grain:
`signing_secret`, `bot_token` and `team_id` are channel config, so two Slack channels on
one agent are two installs with independent credentials rather than a collision. See
[slack-modernization.md](slack-modernization.md) for what the flow still has to decide.

**The manifest route is channel-scoped and its publish gate narrows**, per
[Publish and expose](#publish-and-expose). Each channel serves a manifest naming its own
request URL, gated on its own `status`; moving one channel's publish state leaves its
siblings where they were, which is what stops "publish to get a manifest" from exposing an
anonymous chat surface next to it.

**Transport-typed config is confirmed, not questioned.** `agent_surface_enabled` governs
manifest and event subscriptions, the delivery surface is detected per event, and pane
streaming with `report_progress_only` on channel threads is a per-surface answer. None of
that generalizes across transports. Shared exposure *policy* (tool visibility) moves to
the neutral module; transport config stays typed per transport.

## Integrations tab

Agent detail owns an **Integrations** tab alongside Overview / Preview /
Credentials / Triggers / Versions / Stats:

- Stat strip: Health / Invocations 24h / Success rate / Activity, unchanged.
- **Channels** section: channel inventory with create, edit, publish, unpublish, delete,
  and migrated schedule run-now controls.
- **Triggers** section: absorbs today's Triggers tab. Read-only UI renders a
  human-readable cron description plus timezone; raw cron only inside the editable input.
- Header: agent identity control and the suspend-all-exposures switch.
- App editor routes are retired. Channel and trigger editors are Agent-owned.

`Credentials` stays the org-wide read view of channel keys and tokens.

The `/apps` list page does not simply disappear. Its real job is answering "what in this
org is reachable from outside right now", which is a question security asks and no agent
detail page answers. It becomes a read-mostly cross-agent **Exposures** view that links
into agents (EVE-1010, landed at `/exposures`).

**The word is "Exposures"**, settling open question 3. The API and CLI already say it
user-facingly (`/v1/agents/{id}/exposures/suspend`, `everruns agents exposures suspend`),
so a different UI term would make the product say two things about one concept; and it
covers triggers, which "Channels" does not. It sits under **Operational**, beside
Sessions and Reports, because reading it is an operational act and the editing it links to
lives on the agent.

The view **resolves** state rather than reading `channel.status`: it folds in the
agent-level terms the same way `app_ingress::channel_liveness` does, so a live channel on
a suspended or archived agent never reads as Live. Anonymous *configuration* and *live*
reachability are reported separately — an anonymous channel says so while it is still
draft or suspended, because resuming its agent opens it and the row has to warn before
that, not after.

## Migration

Nine independently shippable phases. Phases 0 and 2 are worth landing whether or not the
rest proceeds.

0. **Hide App from the product surface** (EVE-998). Remove the nav entry and the create path so no
   new Apps are made while the model moves. Reversible, no migration, lands immediately.
1. **Resolve the grandfathered agent-less Apps** (EVE-999). [apps.md](apps.md) deliberately does not
   backfill them, and they block every later phase. Synthesize a hidden Agent per row
   (`system_prompt = ""`, `harness_id = app.harness_id`), which is what such an App already
   means at runtime: the harness with no agent overlay.
2. **Channel-scoped ingress routes** (EVE-1000), mounted alongside the app-scoped ones. No model
   change. Fixes the AG-UI addressing gap on its own.
3. **Reserve the new tag prefixes** (EVE-1002) before anything writes them, and keep the old ones
   reserved forever. Must precede phase 4.
4. **`agent_channels`** (EVE-1003, landed) with an `agent_id` FK, backfilled from
   `app_channels ⋈ apps`. `app_channels` is now a read-only view over `agent_channels`,
   retained for compatibility; every writer targets the table. `status`, `agent_identity_id`,
   `agent_version_policy`/`agent_version_id`, and `owner_principal_id`/
   `resolved_owner_user_id` are first-class channel columns. The `auth` config did not
   move: it lives inside the channel-config encryption envelope, so lifting it is its own
   migration (EVE-1019).

   **4b — session and budget attribution** (EVE-1004, landed). `sessions.channel_id` and
   the `agent_channel` budget subject follow the exposure down from the bundle. Sessions
   are attributed from the channel routing tag the server itself wrote — three spellings,
   because the convention grew per transport — and from an App's sole channel when it has
   only one; anything ambiguous stays NULL rather than being guessed into a door it may not
   have come through. `app_channel` budgets convert 1:1 (the channel kept its `appchan_`
   id, so only the subject type's name moves). `app` budgets have no 1:1 successor and fan
   out to one budget per channel, **preserving** the cap rather than dividing it: dividing
   would tighten every existing cap without consent, and the App budget stays enforced
   alongside, so the original ceiling keeps binding. `sessions.app_id` and the
   `app`/`app_channel` subject types remain permanent historical attribution.
5. **Unify the binding enums** (EVE-1005); move webhook from channel to trigger type
   (EVE-1006).
6. **Per-channel publish**, `agent.exposures_suspended`, stop reading `App.status`
   (EVE-1007, landed). Every ingress gate resolves liveness through one helper,
   `app_ingress::channel_liveness`. The App publish switch remains, and now drives the
   channels it owns, until App management is retired. The Slack manifest and bot identity
   move to the channel separately (EVE-1008).
7. **UI**: Integrations tab with channel and trigger editors (EVE-1009), cross-agent
   Exposures view (EVE-1010).
8. **Freeze** the `apps` table and retire App management (EVE-1011). Historical records,
   attribution, budget subject values, and permanent route aliases stay. Ingress and
   Agent channel management no longer read or write Apps.

## What this costs

- **Vocabulary.** "App" is a word customers use. Against that: it collides with the Slack
  app they also create, and what they built is an agent that is reachable in Slack.
- **Bundle editing.** One App configures harness, agent, and identity once for N channels.
  After the move, identity and version policy repeat per channel. Mitigated by making both
  nullable with agent-level defaults — and the reverse case (two channels, two identities)
  is impossible today without two Apps.
- **A migration that touches security invariants.** Ownership override, reserved tag
  prefixes, and tag containment all have threat-model entries. Phase 2 is the risky one and
  should carry the reuse and tag-spoofing tests before the backfill, not after.

## Open questions

1. ~~Does `Requester` binding mean the platform user, the external actor, or both keyed
   together?~~ **Settled by EVE-1005: the transport's own external actor id, always.** A
   Public Chat visitor is anonymous or Google-signed-in and unrelated to any Everruns
   account, so keying on a platform principal would be undefined for exactly the surface
   that most needs it. The `{platform}:` prefix already namespaces the id, so one rule
   covers every transport without splitting the variant.
2. Should a channel be allowed to point at an agent in a *different* org-visible scope
   (shared agents), or does the channel always live with its agent?
3. ~~Is `Exposures` an ops page or a nav-level concept?~~ **Settled by EVE-1010: nav-level,
   under Operational, and the word is user-facing.** It is the only cross-agent surface
   the design keeps, so it was the surface that decided whether "exposure" became user
   vocabulary.
4. ~~EVE-978 (suggested prompts) picks a source per surface.~~ Settled: **agent config**,
   falling back to the harness, resolved by `everruns_platform::exposure::resolve_starters`
   over the `starters` field Platform Chat already uses. Channel config was not available
   to choose — it does not exist until EVE-1003 — and a channel-level override remains
   strictly additive on top of that order, so nothing here is foreclosed.

## Naming: Endpoint became Channel

**2026-09-23.** The agent-owned door shipped as an **Endpoint** and is now a **Channel**,
end to end: product copy, UI routes (`/agents/{id}/channels/…`), management API
(`/v1/agents/{agent_id}/channels/…`), Rust types, the `agent_channels` table,
`sessions.channel_id`, and the `agent_channel` budget subject. "Endpoint" reads as a URL,
and a Slack workspace install is not a URL — it is a channel the agent is reachable
through. "Port" was considered and rejected as equally technical. The collision with the
retired App's `app_channels` is acceptable because that concept is the one being
replaced: an App channel *is* what became an agent channel.

What deliberately did not move, because renaming it would break live installs or
orphan sessions:

- Public ingress paths `/v1/e/{channel_id}/…` and the permanent `/v1/apps/…` aliases.
- Persisted routing tags `endpoint:{id}`, `slack:endpoint:…`, `fcp:endpoint:…`, which
  are load-bearing for session reuse (see [Session binding](#session-binding)).
- The `api_endpoint` channel type, which names an API endpoint and is correct as is.
- The `SessionBinding::Endpoint` variant and its `endpoint` read alias, which are
  internal and serialize as `shared_session` anyway.
- `agent_endpoint` is still accepted as a budget subject type on input, so existing
  API callers keep working.

The table rename ships with a one-release compatibility view and a
`sessions.endpoint_id`/`channel_id` sync trigger so pods on the previous build keep
working during a rolling deploy; a follow-up migration drops both
(`crates/server/migrations/144_rename_agent_endpoints_to_channels.sql`). MCP, provider,
and OAuth "endpoints" are unrelated and keep the word.

## References

- [Apps](apps.md), the model this replaces
- [App Invocation Channels](app-invocation-channels.md), webhook semantics and the session-ownership override
- [Channel Authentication](channel-auth.md), the auth config that moves to the channel
- [App API Keys](app-api-keys.md), the exposure-as-boundary rule
- [Agent Triggers](../runtime-resources/agent-triggers.md), the trigger model that absorbs webhook
- [Messaging Integrations](messaging-integrations.md), adapter lifecycle and routing tags
- [Slack Integration Modernization](slack-modernization.md), per-event surface selection and what remains
- [Concepts](../foundations/concepts.md), the Harness/Agent/Session layering this leaves intact
