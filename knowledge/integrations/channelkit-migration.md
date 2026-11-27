---
type: Architecture
title: Channelkit migration
description: Move Slack provider operations to Channelkit while retaining Everruns channel orchestration, then qualify additional messaging providers independently.
tags:
  - everruns
  - integrations
  - channels
  - slack
---

# Channelkit migration

## Scope and dependency

Adopt Channelkit **below the existing channel host**, through a bridge in
`everruns-integrations`. Keep Everruns' session, turn, identity, authorization and
durability contracts. Use Channelkit's adapters directly; do not introduce its
in-process `Channels` coordinator alongside `ChannelHost`.

Slack is the first migration and the parity bar. Other providers follow one at a
time. A2A, AG-UI, Poppy, voice, public chat and the Agent Execution API retain their
own protocol contracts.

The bridge pins the published Rust `0.1.0-alpha.7` core and Slack packages, frozen at
[`c24f799`](https://github.com/everruns/channelkit/commit/c24f799115efb1939c60d849c9cfee052287dddd).
The [versioned Slack guide](https://docs.rs/channelkit-slack/0.1.0-alpha.7/channelkit_slack/guide/)
and upstream [adoption contract](https://github.com/everruns/channelkit/blob/c24f799115efb1939c60d849c9cfee052287dddd/knowledge/operations/everruns-adoption.md)
own library behavior. The shared pin also matches the server's AgentMail adapter,
avoiding incompatible core types across transports without another upstream release.
Cargo resolves registry artifacts with checksums; no mutable
main or path replacement participates in the deployment build.

## Implementation and qualification

The shared bridge now owns Framework, serve and server Slack provider operations.
Channelkit verifies/parses ingress and owns formatting, native streams, cards,
history and media. HTTP and trusted Socket Mode share normalization; Everruns keeps
socket connection supervision and app provisioning/OAuth lifecycle. Privacy-scoped
user projection and reactions use Channelkit's trusted API escape hatch.

The server retains response policy, durable approvals and explicit-action claims.
New approval cards bind the original reply destination for both public and private
placement, checked before the durable claim. Older signed public cards retain their
previous wire format; private cards still require the original destination.
Approval freshness and consumption follow the session's authority: v1 uses its
database transaction; v2 reads the canonical log and atomically claims its checked
position on the shard. All renderings of one request share that claim, retained
through event offload, snapshots and moves. A concurrent input invalidates the
read, and missing or unavailable v2 authority never falls back to v1.
The internal claim command requires the existing
[matched control-plane/shard upgrade contract](../execution/data-plane-v2/design.md#upgrading-code):
pause v2 writes and upgrade both together; an older shard cannot consume the new
claim records. Follow the [cutover runbook](../../docs/sre/runbooks/channel-delivery-cutover.md).
Native history context preserves subtype filtering. Identity and thread reads use
Slack GET query parameters; native files and standalone actions retain host-scoped egress. Explicit posts use the single
message API, including full plain fallback only when it fits, and never retry an
uncertain acceptance. Worker RPC preserves partial receipts and retry advice without
transferring bot credentials.

The Framework `channels` feature enables its default policy-aware host egress;
otherwise the provider model can finish while Slack delivery remains offline.
An application-supplied egress service remains authoritative for every provider hop.
Native file downloads accept the host ceiling and cap transport reads instead of
rejecting a larger budget before fetching a small file. This preserves the host's
default 25 MiB intake contract; upload limits remain provider-specific.
File qualification includes model access after intake: Responses encoding must
carry native file parts correctly through user input, tool output and compacted
history. A successful provider download alone does not prove the agent can read it.

The output checkpoint foundation is merged. Opting drivers use a fenced write-ahead
ledger in Memory, serve SQLite and server PostgreSQL; confirmed split parts and native
handles survive restart. Unknown sends remain pending. New pending rows carry a
protocol marker; unmarked legacy rows without a readable ledger require operator
reconciliation rather than automatic event replay. See the
[channel contract](channels.md#output-delivery-checkpoints) and
[cutover runbook](../../docs/sre/runbooks/channel-delivery-cutover.md).

Local deterministic bridge, signature, scope, receipt, native-file and history
qualification is tracked with the source tests and the
[acceptance case](../test-cases/agents/slack_channels/TC001_migration_parity_and_recovery.md). Final server parity, real-model
turns, native user-origin messages/approval clicks and deployed-host adoption are
separate gates; do not infer them from library live API checks or signed fixtures.
The dependency publication does not itself complete the Everruns cutover.

## Existing architecture and the boundary to retain

[Channels](channels.md) already separates provider wire operations from the
channel host, session port, store and `TurnDelivery`. Framework, serve and server
share that host. Moving transport code should strengthen this boundary.

| Owner | Responsibilities after migration |
| --- | --- |
| Channelkit | Provider authentication/parsing; scoped provider conversation/message references; outbound HTTP, formatting, limits, media operations, native history and honest provider errors/capabilities |
| `everruns-integrations` bridge | Map provider events/references/receipts into Everruns contracts; adapt delivery probes and typed provider extensions; host-injected egress transport |
| Everruns core | Session binding, event dedup, admission/steering, participant attribution, delivery policy, guardrail replacement, terminal notices, recovery and receipt checkpoints |
| Everruns server | Tenant/channel resolution, encrypted secrets, publication/liveness, response policy, app provisioning, OAuth consent, approval authorization/audit, health, quotas and budgets |
| Framework / serve | Application-owned credentials and storage, HTTP mounting, lifecycle tasks and the same bridge |

Never pass Slack tokens to the worker or model. Preserve the session-bound action
seam described in [Slack Agent Actions](slack-agent-actions.md). Channelkit objects
stay in the process that owns the credentials. Cache adapters by channel identity
and credential revision, with bounded eviction; do not cache by workspace alone.
Persist only the credentials needed by a particular interaction, encrypted and with
expiry. Treat private response URLs and interaction tokens as secrets; validate
provider destinations and reject redirects before using them. Never store bearer
headers in the inbox or trust a webhook body to select an outbound credential.

## Initial Slack parity analysis

Everruns has more than a mention-to-reply bot. Its implementation, rather than older
Slack overview prose, is the reference for behavior. This original fit/gap analysis
explains the upstream work below; the pinned dependency now supplies those
mechanics. Everruns retains the host policies and qualification responsibilities.

| Behavior | Channelkit fit / gap | Migration requirement |
| --- | --- | --- |
| Raw-byte HTTP signature verification, replay window, URL challenge, workspace scope | Supported by Slack ingress | Preserve missing/bad-signature and team/channel isolation cases; return the library challenge only after host route/liveness checks |
| Message/mention duplicate delivery, subtype filtering, attachment-only input | Supported, with different normalized event identities | Map to existing Everruns dedup keys; retain both mention and message fixtures and existing supported-subtype policy |
| Session reuse, requester/conversation/thread strategies, pane forcing thread binding, inline DMs | Host policy, not adapter dispatch | Retain existing binding keys and per-turn targets; do not use Channelkit subscriptions as routing authority |
| Legacy attachment/unfurl summaries and external-file descriptions | Normalized file metadata alone is insufficient | Extend typed Slack ingress projection before removing the existing parser; preserve summaries, image links and attachment-only unfurls |
| Private file retention and modern file upload/download | File operations supported | Use host egress and retention limits; test external files, denied hosts/redirects, PDFs, compaction and token redaction; avoid moving bytes across the worker seam unnecessarily |
| User display names and participant attribution | User lookup supported | Keep scoped cache/error policy; actors remain untrusted display data, never Everruns account authority |
| Ambient-message relevance gate and thread backfill | Host admission plus paginated history | Keep explicit-invocation bypass, fail-silent classifier and bounded thread-local history; exclude the triggering message and report truncation |
| Plain/Markdown replies, long replies and task-progress edits | Supported primitives | Preserve notification fallback, current formatting, disabled unfurls, broadcast neutralization and confirmed references for every split part |
| Pane native streaming and recipient context | One-shot stream API does not match `ChannelStreamDelivery` | Upstream incremental start/append/stop with serializable scoped handles; bridge replacement by successful stop then edit; host owns cadence and all terminal cleanup |
| Stop, title and viewing-context controls | Normalized controls supported | Keep actor authorization, channel binding and rename no-op suppression; missing actor is informational, not a cancellation grant; qualify old/current Slack method compatibility |
| Approval cards, requester-only decisions and private approval placement | Generic cards/actions are not the complete existing contract | Preserve value/correlation binding, private/ephemeral delivery, source message/container references, resolved-card updates and exact requester/channel/session checks |
| Bidirectional reactions and stored-message correlation | Reactions and explicit removal supported | Translate names/custom emoji consistently; keep durable mapping from each platform message to the corresponding stored message; feedback never grants approval |
| MCP App picture/link fallback | Upload and post are primitives, rendering is host-specific | Keep snapshots, owner access and links in Everruns; qualify button/link payload support before deleting current wire helpers |
| Scheduled posts and explicit `send_message` | Provider posting supported; destination policy is host-owned | Retain trusted input-derived target and durable action ownership; no model-selected sibling bot, channel or thread |
| Socket Mode | No complete Slack socket lifecycle in the initial API | Keep connection management/supervision in Everruns; add a bounded, explicitly host-authenticated normalizer upstream instead of forging HTTP signatures |
| Per-agent app identity, manifest, manager consent, OAuth, branding and health | Baseline manifest is insufficient for Everruns provisioning | Keep management here; provider helpers may build wire payloads, but must not own tenant state, consent or install lifecycle |
| Partial/unknown delivery, rate limits, crash recovery | Channelkit has richer errors and receipts than `DeliveryResult` | Extend Everruns delivery/checkpoint contracts before switching sends; preserve unknown acceptance and all confirmed split-message references |

Source anchors: [server Slack driver](../../crates/server/src/channels/slack/host.rs),
[shared Slack driver](../../crates/integrations/src/slack_channel/mod.rs),
[turn-specific surfaces](../../crates/server/src/channels/slack/delivery/turn_adapter.rs),
[Socket Mode](../../crates/server/src/channels/slack/socket_mode.rs),
[interactivity](../../crates/server/src/channels/slack/events/interactivity.rs),
[scheduled conversations](../../crates/server/src/channels/slack/scheduled.rs),
[reaction mapping](../../crates/server/src/channels/slack/delivery/reactions.rs),
[Channelkit adapter contract](https://docs.rs/channelkit/0.1.0-alpha.7/channelkit/trait.Adapter.html)
and [Slack guide](https://docs.rs/channelkit-slack/0.1.0-alpha.7/channelkit_slack/guide/).

## Correctness prerequisites

**Installation identity is the Everruns channel identity, not the Slack team.**
Two agent-owned Slack apps in one workspace are different credential principals.
Map Channelkit's provider/installation/channel/thread reference to that stable
identity. Preserve `slack:thread:*`, `slack:channel:*` and `slack:user:*` binding keys
for existing sessions; do not silently re-key from Channelkit serialization.
Keep fractional Slack timestamps as strings. Namespace new provider bindings and
actors by installation; external user IDs must not link people across tenants.

**An ACK is not durable acceptance.** Current Slack routes acknowledge before
background host processing, and Socket Mode acknowledges envelopes promptly. The
migration must document that existing crash window rather than claiming that library
parsing removes it. For a stronger guarantee, persist a bounded tenant-scoped inbox
and dedup identity before ACK, then drain through the existing host. Keep this a
separate, measurable durability change with overload/failure responses within the
provider deadline. Do not add a second autonomous worker inside Channelkit.

**Do not flatten provider errors into generic transient failures.** The bridge
preserves structured retry advice and partial/ambiguous acceptance, with confirmed
message references and a per-output-part checkpoint. A rate-limit rejection can be retried after provider
advice; unknown acceptance must remain pending for reconciliation. Never replay
all parts after some were accepted. A checkpoint written after a remote send cannot
close the send/commit crash window: record intent first, mark uncertain on interrupted
attempts, and reconcile without claiming exactly-once remote delivery. A confirmed
stream reference is not proof that its last append succeeded.

**Shared host recovery needs qualified output checkpoints.** Pending input and
event cursors alone cannot establish remote-output acceptance. The host combines
those records with a write-ahead output ledger. Qualify crashes before dispatch,
after acceptance/before checkpoint, between split parts,
mid-stream and during stop/edit replacement. Retain lease fencing and both data-plane
session ports; a steered input must not acquire a second reply delivery.

**Capabilities are per target and operation.** A Slack app's normal channel and
agent pane have different affordances. An adapter-level `Conditional` flag alone
cannot enable streaming, status or private cards for every target. Preserve
`TurnDeliveryPlanner` and explicit probes, with tested degradation where the product
already supports it. A provider's unsupported approval surface must not advertise
the client hint that pauses for an unrendered card.

**Retraction and mention safety survive every path.** Keep Everruns' buffered
incomplete mention token and broadcast-neutralization policy across ordinary posts,
stream appends, edits, status/progress and plain-text fallback. Channelkit formatting
support does not authorize `@channel`, `@here` or user-group broadcasts. Keep raw
provider escape hatches in trusted provider code, never expose them as model tools.

## Upstream work packages

The published dependency implements these packages. The boundaries below are the adoption
contract; provider-origin interaction and recovery qualification still belongs to
the consuming Everruns implementation.

1. **Incremental Slack streams.** Add explicit scoped start/append/stop, optional
   recipient context, split-without-truncation and ambiguity preservation. A host
   stores the handle and owns cleanup/retraction. No background worker or DB.
   Published upstream in the dependency above; adoption still needs the
   consumer delivery and recovery gates below.
2. **Authenticated Slack ingress projection.** One parser for verified HTTP and
   host-authenticated Socket Mode payloads; strict workspace scope and bounds on
   both. Preserve typed subtype/channel/surface context, external-file/unfurl data,
   and ephemeral interaction container references. Public HTTP must never select
   the trusted path by a request field.
3. **Slack message/action extensions.** Typed options for host correlation metadata,
   independently bound action values, links, ephemeral/private posts and resolved
   card edits. Keep generic text/card APIs small; do not add Everruns session IDs,
   approval policy, snapshot renderer or OAuth database to Channelkit.
4. **Qualify existing methods.** Check modern/compatibility status/title methods,
   reply/backfill ordering and scope, file URL policy and permission errors against
   designated test apps. Add a provider helper only for demonstrated missing wire
   behavior. OAuth install, Socket Mode supervision, task-progress state and stream
   rotation are not prerequisites to move wholesale into the library.

Do not bypass these contracts with a permanent parallel Slack HTTP stack. During transition,
inventory every remaining direct call and remove it in the owning phase. A temporary
raw Channelkit API call must be host-scoped, tested and tracked for replacement.

## Dependency-ordered delivery plan

| Step | Change | Completion gate |
| --- | --- | --- |
| 0. Freeze contracts | Record paired Slack fixtures, current bindings/targets, actions and delivery effects; separate public-core and private deployment work | Deterministic parity suite captures current HTTP and Socket Mode behavior, including the latest surfaces |
| 1. Upstream prerequisites | Land the four focused packages above with crate guides and negative HTTP tests | Upstream local checks and CI green; exact candidate dependency available; no unsupported capability claims |
| 2. Everruns delivery outcomes | Structured errors, receipt/part checkpoints, intent/uncertain state and retry/reconciliation rules | Crash and multi-replica tests prove no blind resend of unknown/confirmed parts; no claim of remote exactly-once |
| 3. Shared bridge | Feature-gated Channelkit adapter in integrations, target/actor/event conversions, injectable transport and scoped factory | Contracts/core do not acquire provider HTTP dependencies; bridge tested against real loopback HTTP |
| 4. Framework and serve Slack | Replace their Slack wire implementation with the bridge, including history, actions, controls, approvals and advanced surfaces | Existing suites plus parity fixtures pass; restart cases exercised with application and SQLite stores |
| 5. Server outbound | Move normal/keyed sends, streams, progress, reactions, files, private cards, snapshots and explicit actions through Channelkit | All existing server Slack behavior passes on both session ports; credential and action ownership unchanged |
| 6. Server inbound | Move signed HTTP, interactions and trusted Socket Mode normalization, retaining admission/backfill/authorization | Signed route-to-session tests; duplicate mention/message, cross-tenant, private approval and reconnect tests pass |
| 7. Cutover and deletion | Canary by stable channel identity, one delivery owner; remove old parser/client/formatting implementations | Real provider-origin inbound, reply/readback, files, stop/rename, approvals, reactions, schedule and recovery evidence recorded |
| 8. Cloud adoption | Adopt merged OSS via the SaaS repository's existing pin/dependency workflow | Wrapper API/UI and deployed-runtime checks; private deployment facts remain in the SaaS knowledge bundle |
| 9. Next provider | Add one adapter/configuration/UI surface through the same bridge and host | Provider-specific acceptance matrix complete before general availability |

Deliver each step as small independently tested changes under its repository's
workflow. Inbound and outbound qualification remain separate evidence, and the same conversation
always has one delivery owner.
Outbound shadow mode is prohibited because it sends duplicates. Parser comparison
may run without effects using sanitized fixtures; live comparisons require the
host's existing privacy/retention policy and must never launch two turns.

Rollback keeps channel/session/binding IDs, the output ledger and credential revision.
Keep the persisted `slack` channel type, existing configuration and permanent legacy
ingress aliases. Channelkit is an implementation dependency, not a new customer-facing
channel type or a reason to issue new webhook URLs.
Drain or fence the old owner before changing implementation. Unknown sends remain
pending for reconciliation across rollback; switching the adapter is not permission
to resend them. No Slack app reinstall or OAuth scope expansion should be necessary
solely to change the implementation.

## Qualification and rollout measurements

- Ingress: invalid/tampered/stale signatures; malformed/oversized input; wrong team,
  configured channel, actor or tenant; mention/message duplicates; Socket Mode
  redelivery/reconnect; attachment-only and legacy-unfurl-only messages.
- Session behavior: existing session continuity; same-team different-app isolation;
  root/thread/DM/pane targets; every offered binding; mid-thread history limits;
  admission failure/timeout, steering and session-title echo suppression.
- Delivery: Unicode and fenced Markdown splitting; disabled broadcasts and unfurls;
  definite rejection versus rate limit versus unknown acceptance; confirmed-part
  recovery; stop after failure/cancel; close-before-replace; no empty-turn silence
  unless `no_reply` intentionally suppresses it.
- Actions: requester/private-card authorization, stale/double clicks, exact
  session/channel/thread binding, reaction add/remove, snapshot fallback and
  scheduled/explicit posts through the correct bot; no token in diagnostics or RPC.
- Host integration: Framework, serve and server; both server session ports;
  multi-replica leases, credential rotation, unpublished/deleted channels, real
  HTTP acceptance deadlines and bounded work under downstream backpressure.

Track ACK latency/deadline failures, duplicate inputs/turns/posts, binding misses,
pending-delivery age, unknown outcomes, partial sends, rate-limit delay, leaked/open
streams, authorization refusals and provider error codes by provider/channel revision.
Agree canary thresholds from the captured baseline before rollout; zero duplicate
turns, cross-scope writes and unauthorized decisions are correctness gates. Synthetic
signed fixtures, API acceptance, readback and provider-origin E2E are separate evidence.

## Additional channels after Slack

Do not translate every provider into Slack's threading or approval model. The shared
bridge maps native identities; capability probes choose the supported experience.
Start with **Discord** as the default next candidate, since it exercises a distinct
Gateway and interaction-reply lifecycle. Choose **WhatsApp** first instead if customer
requirements prioritize it; that is a product sequencing decision, not a library
readiness assumption. Implement the selected provider after the Slack cutover gate.

| Candidate | Specific qualification required |
| --- | --- |
| Discord | Host Gateway identify/resume/intents; native thread-as-channel identity; signed interactions and deferred replies; encrypted interaction-token retention; explicit limits for missing media/DM features |
| WhatsApp | Business/phone installation scope; signed batches; reply-window context across restarts; explicit approved templates and opted-in recipients for proactive posts; delivery/read statuses separate from acceptance; no edit/history fiction |
| Microsoft Teams | Tenant/app/service-URL binding, fresh trusted signing keys, token refresh, native activity/reply identity and authorized Adaptive Card actions; package availability and real tenant qualification |
| GitHub / Linear | Reuse their existing Everruns connection identities and triggers; distinguish invocation from conversational replies so one event does not start work twice; preserve namespaced comment/session identities and native activity limitations |
| X / Web | Separate product scope and readiness review; Web is not a reason to replace existing AG-UI/public-chat security or state contracts; verify publication and provider-origin evidence independently |

Every new provider needs management/API/schema/UI registration, encrypted credential
resolution, installation-scoped bindings, actor policy, capability-specific hints,
health and docs in addition to an adapter. Keep native `slack` tools available during
migration; add common tools through the existing conversation sender seam without
inventing unsupported provider actions.

## Rollout and next-provider decisions

- Select the next provider from actual customer demand after Slack parity.
- Keep the exact qualified registry version pinned through cloud adoption.
- Durable-before-ACK intake remains a separate reliability increment. The existing
  acknowledge-before-background-processing crash window is unchanged.
- Set operational canary size, duration and rollback thresholds from baseline
  measurements. These are rollout decisions, not reasons to delay the bridge design.

Related contracts: [Messaging Integrations](messaging-integrations.md),
[Slack response policy](slack-response-policy.md),
[Explicit Communication](explicit-communication.md), and
[Framework application services](../framework/application-api.md).

## Rolling-upgrade baseline

Before enabling the Slack bridge on a rolling deployment, every delivery host
must understand the output protocol marker as well as existing ledgers. A legacy
adapter must retain marked pending rows in the pre-intent crash window, and a
requiring adapter must retain unmarked legacy rows that have no audited ledger.
Missing channels never prove nonacceptance. Land and deploy this shared-host
compatibility baseline first, confirm all old owners are gone, then adopt the
bridge. Rollback uses that baseline or a later protocol-aware build. See the
[output contract](channels.md#output-delivery-checkpoints) and
[cutover runbook](../../docs/sre/runbooks/channel-delivery-cutover.md).
