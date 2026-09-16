---
type: Decision
title: "Agent Exposure (retiring the App abstraction)"
description: "Make Agent the addressable entity by re-homing channels as Endpoints and folding invocation into Triggers, retiring App."
tags:
  - everruns
  - integrations
  - agents
  - apps
---
# Agent Exposure (retiring the App abstraction)

> Status: **Accepted, partially implemented.** The direction is decided; the phases are
> tracked as separate issues in Linear (OSS project, EVE team), EVE-998 through EVE-1011.
> Phases 0, 1 and 4 have landed: App is hidden from the product surface, grandfathered
> agent-less Apps have synthesized Agents, and channels now live in `agent_endpoints`
> owned by an Agent.
> [apps.md](apps.md) remains the accurate description of what exists today and stays
> authoritative until the final phase removes it.
>
> One decision beyond the original proposal: **App is hidden from the product surface
> before it is deleted.** Hiding is reversible, needs no migration, and stops new Apps
> accumulating while the model moves underneath. Deleting the table stays last.

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

- **Endpoints** — a door. Something outside connects, and waits for an answer on the
  same transport. Slack, AG-UI, Public Chat, A2A, FCP, API endpoint.
- **Triggers** — an alarm clock. Something fires, a message is injected, nobody is
  listening. Schedule (exists today), webhook (moves here from channels).

`apps` is then deleted. No ingress URL changes for anyone already installed, because
endpoint identity moves *off* the owning entity and onto the endpoint itself.

## Why an Agent can absorb this now

Four App responsibilities have to land somewhere. Each already has a natural owner one
level down:

| App owns today | Where it goes | Why it survives the move |
|---|---|---|
| `harness_id` | deleted | The agent already pins one; App's copy is a second source of truth that the create form fills *from* the agent. |
| `agent_id`, version policy | agent is the parent; policy moves to the endpoint | A staging endpoint on `latest` and a prod endpoint `pinned` is a real case the App forces into two Apps. |
| `agent_identity_id` | endpoint, defaulting to the agent's | Two channels needing different identities requires two Apps today. Per-endpoint is strictly more expressive. |
| `owner_principal_id` | **endpoint, mandatory** | Load-bearing for security. See [Invariants that must not move](#invariants-that-must-not-move). |
| `status: draft/published` | per-endpoint status + one agent-level suspend | A "published" App with three draft channels is a lie the 2-D `App.status × channel.enabled` matrix tells today. |

The three structural objections to collapsing App into Agent — N exposures per agent,
`agent_versions` immutability, and the App-as-security-boundary rule in
[app-api-keys.md](app-api-keys.md) — are objections to losing the **per-exposure row**,
not to losing the App. Every one is satisfied by an `agent_endpoints` row: the row is the
boundary an API key scopes to, the row is what publish toggles, and the row is what stays
out of the agent's config snapshot.

## Model

### Endpoint

One row per way in. Transport-typed config, exactly as `app_channels` is today — the
config stays a per-transport typed union (`SlackChannelConfig` and friends in
`crates/platform/src/app.rs`, moving to `crates/platform/src/endpoint.rs`), because
`agent_surface_enabled` has no meaning for A2A and pretending otherwise produces a
lowest-common-denominator config that fits nothing.

What is *not* per-transport, and becomes a first-class column rather than a field inside
each config variant:

- `status`: `draft | live | disabled`
- `session_binding` (see [Session binding](#session-binding))
- `agent_identity_id`, nullable, defaults to the agent's lazy identity
- `agent_version_policy` + `agent_version_id`
- `owner_principal_id`, `resolved_owner_user_id`
- `auth`, the shared inline config from [app-endpoint-auth.md](app-endpoint-auth.md)

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

### Why split Endpoints and Triggers at all

The question is not whether to split, it is **where to cut**. Other platforms cut
`channels | automations`, which keeps webhook on the channel side because it arrives over
HTTP. Cut on *transport* and the two groups have overlapping, half-empty field sets —
which is exactly the state `app_channels.channel_config` is in.

Cut instead on **is there a peer waiting for a reply**, and the field sets become
disjoint:

| | Endpoint | Trigger |
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
| `Endpoint` | the endpoint row | `shared_session` |
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
`{transport}:{binding}:{key}`, with `agent:{agent_id}` and `endpoint:{endpoint_id}`
replacing `app:` and `app_channel:`.

## Publish and expose

Publish moves **to the endpoint**, with one agent-level override:

```
live(endpoint) = endpoint.status == live
              && agent.status == active
              && !agent.exposures_suspended
```

- Per-endpoint `status` is the everyday control: publish the Slack endpoint without
  flipping on the public chat endpoint sitting next to it.
- `agent.exposures_suspended` is the incident control — one switch, take the agent off the
  internet — which is what App unpublish is actually reached for.
- `agent.status != active` must imply no live endpoint. Enforce at resolution time, not by
  writing rows, the same way the harness chain is folded behind the platform seam.
- Agent exposure state for lists and badges is **derived** (`any endpoint live`), never
  stored. A stored flag is a second writer.

This is a security improvement, not just a tidier lifecycle. Today, obtaining a Slack
manifest requires publishing the whole App (EVE-970 sequenced publish before Slack app
creation), which simultaneously exposes every other channel on it. Per-endpoint publish
narrows that gate to the endpoint that needs it.

## Endpoint identity and URLs

The hard constraint: ingress paths are embedded in Slack app manifests, A2A agent cards,
integrator code, and public chat links. Re-pointing them at `{agent_id}` breaks live
installs.

It does not have to change, because the `app_id` segment in those paths is already
redundant — the channel id is globally unique and already appears in half of them
(`crates/server/src/api/`). Mount ingress on the endpoint alone:

```
POST /v1/e/{endpoint_id}/slack/events
POST /v1/e/{endpoint_id}/ag-ui
POST /v1/e/{endpoint_id}/a2a
GET  /v1/e/{endpoint_id}/.well-known/agent-card.json
POST /v1/e/{endpoint_id}/sessions
GET  /c/{endpoint_slug}
```

Old `/v1/apps/{app_id}/…` paths stay mounted permanently as aliases that resolve the
endpoint and ignore the app segment. Nothing installed breaks, ever. Deprecate in docs,
not in code.

Worth doing on its own merits: `/v1/apps/{app_id}/ag-ui` carries no channel segment, so an
App cannot expose two URL-distinguishable AG-UI channels. Endpoint-scoped routes fix that
uniformly instead of per transport.

## Invariants that must not move

These are load-bearing for security and will break quietly if the migration treats them
as bookkeeping.

**Endpoint-owned session ownership.** App-channel sessions adopt
`app.owner_principal_id`, not the caller's — internal callers default to the system
principal, and `shared_session` reuse keys on the owner, so without the override reuse
never matches and ownership is unaccountable. `owner_principal_id` must live on the
endpoint row, not be collapsed to the agent. See
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

**Per-endpoint bot identity is the shape the parked OAuth work needs.** The deferred
install flow is blocked on revisiting the per-App bot identity decision. One workspace
install maps to one endpoint, never to a bundle of channels — the App was never the
natural owner of a Slack install. This design hands that decision the right grain instead
of complicating it.

**The manifest route becomes endpoint-scoped and its publish gate narrows**, per
[Publish and expose](#publish-and-expose).

**Transport-typed config is confirmed, not questioned.** `agent_surface_enabled` governs
manifest and event subscriptions, the delivery surface is detected per event, and pane
streaming with `report_progress_only` on channel threads is a per-surface answer. None of
that generalizes across transports. Shared exposure *policy* (tool visibility) moves to
the neutral module; transport config stays typed per transport.

## Integrations tab

The App detail page is already the right page, on the wrong entity — a channels-first
operations page with a stat strip, expandable channel rows, a live activity rail, and an
inline agent-identity control. Re-home it as an **Integrations** tab on Agent detail,
alongside today's Overview / Preview / Credentials / Triggers / Versions / Stats
(`apps/ui/src/components/agents/agent-tabs.tsx`):

- Stat strip: Health / Invocations 24h / Success rate / Activity, unchanged.
- **Endpoints** section: expandable rows, per-row publish toggle, per-row identity and
  version policy.
- **Triggers** section: absorbs today's Triggers tab. Read-only UI renders a
  human-readable cron description plus timezone; raw cron only inside the editable input.
- Header: agent identity control, and the suspend-all-exposures switch.
- Editors stay full-page routes, not dialogs, matching the current convention:
  `/agents/{agentId}/endpoints/new`, `/agents/{agentId}/endpoints/{endpointId}`,
  `/agents/{agentId}/triggers/{triggerId}`.

The existing **Integrate** tab (the `IntegrationGuide` snippet page) folds *into* the
expanded endpoint row as a "use it" panel. A snippet that carries the real endpoint URL
and key beats a generic guide, and it removes a tab whose name would otherwise collide
with Integrations.

`Credentials` stays the org-wide read view of endpoint keys and tokens; the endpoint row
stays the only write path. One writer, one reader.

The `/apps` list page does not simply disappear. Its real job is answering "what in this
org is reachable from outside right now", which is a question security asks and no agent
detail page answers. It becomes a read-mostly cross-agent **Exposures** view that links
into agents.

## Migration

Nine independently shippable phases. Phases 0 and 2 are worth landing whether or not the
rest proceeds.

0. **Hide App from the product surface** (EVE-998). Remove the nav entry and the create path so no
   new Apps are made while the model moves. Reversible, no migration, lands immediately.
1. **Resolve the grandfathered agent-less Apps** (EVE-999). [apps.md](apps.md) deliberately does not
   backfill them, and they block every later phase. Synthesize a hidden Agent per row
   (`system_prompt = ""`, `harness_id = app.harness_id`), which is what such an App already
   means at runtime: the harness with no agent overlay.
2. **Endpoint-scoped ingress routes** (EVE-1000), mounted alongside the app-scoped ones. No model
   change. Fixes the AG-UI addressing gap on its own.
3. **Reserve the new tag prefixes** (EVE-1002) before anything writes them, and keep the old ones
   reserved forever. Must precede phase 4.
4. **`agent_endpoints`** (EVE-1003, landed) with an `agent_id` FK, backfilled from
   `app_channels ⋈ apps`. `app_channels` is now a read-only view over `agent_endpoints`,
   kept for one release; every writer targets the table. `status`, `agent_identity_id`,
   `agent_version_policy`/`agent_version_id`, and `owner_principal_id`/
   `resolved_owner_user_id` are first-class endpoint columns. The `auth` config did not
   move: it lives inside the channel-config encryption envelope, so lifting it is its own
   migration (EVE-1019). `sessions.app_id` gains `endpoint_id`, and budget `subject_type`
   gains `agent_endpoint` (`agent` already exists), in EVE-1004.
5. **Unify the binding enums** (EVE-1005); move webhook from endpoint to trigger type
   (EVE-1006).
6. **Per-endpoint publish**, `agent.exposures_suspended`, stop reading `App.status`
   (EVE-1007). The Slack manifest and bot identity move to the endpoint with it (EVE-1008).
7. **UI**: Integrations tab with endpoint and trigger editors (EVE-1009), cross-agent
   Exposures view (EVE-1010).
8. **Delete** the `apps` table and the App domain (EVE-1011). Route aliases stay.

## What this costs

- **Vocabulary.** "App" is a word customers use. Against that: it collides with the Slack
  app they also create, and what they built is an agent that is reachable in Slack.
- **Bundle editing.** One App configures harness, agent, and identity once for N channels.
  After the move, identity and version policy repeat per endpoint. Mitigated by making both
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
2. Should an endpoint be allowed to point at an agent in a *different* org-visible scope
   (shared agents), or does the endpoint always live with its agent?
3. Is `Exposures` an ops page or a nav-level concept? It is the only cross-agent surface
   the design keeps, so it decides whether "exposure" becomes user vocabulary. Settled by
   EVE-1010.
4. ~~EVE-978 (suggested prompts) picks a source per surface.~~ Settled: **agent config**,
   falling back to the harness, resolved by `everruns_platform::exposure::resolve_starters`
   over the `starters` field Platform Chat already uses. Endpoint config was not available
   to choose — it does not exist until EVE-1003 — and an endpoint-level override remains
   strictly additive on top of that order, so nothing here is foreclosed.

## References

- [Apps](apps.md), the model this replaces
- [App Invocation Channels](app-invocation-channels.md), webhook semantics and the session-ownership override
- [App Endpoint Authentication](app-endpoint-auth.md), the auth config that moves to the endpoint
- [App API Keys](app-api-keys.md), the exposure-as-boundary rule
- [Agent Triggers](../runtime-resources/agent-triggers.md), the trigger model that absorbs webhook
- [Messaging Integrations](messaging-integrations.md), adapter lifecycle and routing tags
- [Slack Integration Modernization](slack-modernization.md), per-event surface selection and what remains
- [Concepts](../foundations/concepts.md), the Harness/Agent/Session layering this leaves intact
