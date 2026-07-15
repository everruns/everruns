# Agent Triggers

## Abstract

An **agent trigger** lets an Agent act **proactively** — it wakes itself on a
schedule and runs unattended, without an App in front of it. Triggers are owned
by the agent domain and reuse the existing durable scheduler; they move
*ownership* of scheduled invocation onto the agent, not the machinery.

This is the concrete payoff of the agent-first foundation: because the agent
already owns its harness (P1) and supplies the session host (P2 participants), a
trigger needs no execution decision beyond routing — it renders a message and
starts (or reuses) a session on the agent's own harness, as the agent's own
identity.

Field shapes, SQL, and route handlers live in code — see
`crates/core/src/agent_trigger.rs` (`AgentTrigger`, `AgentTriggerType`,
`ScheduleTriggerConfig`), migration `104_agent_triggers.sql`, the
`crates/server/src/domains/agent_triggers/` domain, and the
`/v1/agents/{agent_id}/triggers` API.

## Model

An agent trigger is an org-scoped row owned by one agent:

- **trigger_type** — `schedule` is the only kind today; the enum leaves room for
  event/webhook triggers later.
- **config** (JSONB) — per-type configuration. For `schedule`
  (`ScheduleTriggerConfig`): `cron_expression`, `timezone` (IANA, default
  `UTC`), `session_mode`, and `message` (also the `{{…}}` template body).
- **session_mode** — reuses `app::InvocationSessionMode`: `shared_session`
  (one durable session the agent returns to) or `session_per_invocation` (a
  fresh session each fire).
- **enabled** — whether the trigger's durable schedule is active.
- **durable_schedule_id** — the backing `durable_schedules` row (nullable;
  managed by the domain, never exposed in the API).
- standard `status` lifecycle (`active`/`archived`/`deleted`) + timestamps.

Ids use the `trg_` prefix (`specs/id-schema.md`).

## Scheduling and limits

The cron expression is normalized to the durable 7-field form and validated to
respect a **minimum interval** and a **per-org enabled-trigger cap**, mirroring
the App schedule-channel limits (`SCHEDULE_CHANNEL_*`). These bound how often an
agent can wake itself and how many active triggers an org can accumulate.

## Durable binding

Creating, enabling, disabling, or deleting a trigger keeps a backing durable
schedule in sync — the same lifecycle the App schedule channel uses
(`sync_schedule_binding_for_channel`), re-homed on the trigger:

- an enabled schedule trigger creates/updates a `durable_schedules` row with
  `target_type = Activity`, `target_name = invoke_agent_trigger`, and
  `target_input = { org_id, agent_id, trigger_id }`;
- disabling or deleting the trigger tears the binding down;
- the durable schedule id is stored back on the trigger row and never surfaced
  through the API.

The durable scheduler (`crates/durable/src/scheduler`) fires the schedule and
enqueues the `invoke_agent_trigger` activity — the identical path App schedule
channels use, so catch-up, concurrency, and reliability behavior are shared.

## Execution

When a schedule fires (or a caller hits the manual
`…/triggers/{id}/trigger` endpoint), `invoke_agent_trigger`:

1. resolves the trigger and its agent, and parses the schedule config;
2. renders the `message` template (`{{agent.…}}`, `{{trigger.…}}`,
   `{{invocation.…}}`);
3. creates or reuses a session per `session_mode`, tagged
   `agent_trigger:{trigger_id}` — `shared_session` looks up the existing tagged
   session for the agent's owner, `session_per_invocation` always starts fresh;
4. runs the session on the **agent's own harness** (P1), with the agent as the
   session's `agent_id` **host** participant (P2);
5. **owns the session by a stable owner principal** so budgets, audit, and
   provenance are attributed to a real owner rather than to `system`;
6. dispatches the rendered message and records an audit event.

The manual endpoint fires exactly one invocation for testing without exposing
the durable schedule id.

## Ownership and provenance

Agents do not yet carry their own identity principal, so trigger sessions are
owned by the **org's default owner principal** (resolved via
`PrincipalService::default_owner_principal` for an internal caller). This keeps
unattended runs correctly attributed and budgeted, and is the stable key the
`shared_session` reuse lookup matches on. Converging this onto a per-agent
identity principal — so each agent acts as itself — is P5
(`specs/agent-identities.md`); this spec will move to that owner when P5 lands.

## Relationship to App schedule channels

Agent triggers and the App `schedule` channel are the same scheduled-invocation
machinery. P4 introduces agent triggers as the owned-by-the-agent home for
proactivity; the App `schedule` channel deprecation and migration onto agent
triggers is a follow-up (see `specs/apps.md` / `specs/app-invocation-channels.md`
when that lands). `SessionSchedule` ("continue this conversation later") and App
**webhook** channels are orthogonal and unchanged.
