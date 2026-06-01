# App Invocation Channels

## Abstract

App invocation channels are unattended ingress modes on top of the App abstraction. They let an App inject a configured user message into an app-owned session when an external trigger fires.

Current invocation channels:

- `schedule` — app-owned durable cron trigger
- `webhook` — authenticated HTTP trigger

This spec is intentionally separate from:

- session-local scheduling concerns
- `specs/scheduled-tasks.md`, which defines the generic durable scheduler
- `specs/app-endpoint-auth.md`, which defines the shared inbound authentication
  framework used by AG-UI and A2A and intended for future App-published
  endpoints. Webhook channels keep using the channel-local `token` field
  described below until their handler is explicitly wired into that verifier.

The durable scheduler is infrastructure. App invocation channels are one product consumer of that infrastructure.

## Goals

1. Let an App run unattended without adding a messaging integration such as Slack
2. Reuse the existing App lifecycle, ownership, harness, agent, and agent identity model
3. Support two invocation routing modes:
   - reuse one stable session per channel
   - create a fresh session per trigger
4. Keep schedule/webhook behavior in the app domain instead of scattering logic across scheduler APIs

## Non-Goals

1. Replacing per-session user-created schedules
2. General-purpose public webhooks without shared-secret authentication
3. Multi-step workflow builders or DAG scheduling

## Model

An invocation channel is stored as a normal `app_channels` row:

- `channel_type = schedule | webhook`
- `channel_config = typed JSON config`
- `enabled = channel-local gate`

Schedule channels also own a managed durable binding:

- foreign key: `app_channels.durable_schedule_id`
- target activity: `invoke_scheduled_app_channel`
- target input: `{org_id, app_id, channel_id}`

See `crates/core/src/app.rs` for canonical domain types.

## Session Routing

Invocation channels use `InvocationSessionMode`:

- `shared_session`
  - one stable session per app channel
  - useful for rolling repo checks, inbox triage, or long-running unattended threads
- `session_per_invocation`
  - fresh session per trigger
  - useful for isolated fanout or request-by-request processing

Shared sessions are tagged with:

- `app:{app_id}`
- `app_channel:{channel_id}`
- `app_channel_type:{channel_type}`

Per-invocation sessions add:

- `app_invocation:{uuid}`

## Session Ownership

App-channel ingress (`webhook`, `schedule`, `ag_ui`, `slack`, A2A) typically runs as `Caller::internal(org)`, whose default principal is the system principal. Without an override, sessions created from an app channel would be owned by `system-owner`, and `shared_session` reuse — which keys on `app.owner_principal_id` — would never match.

To make `shared_session` work and to keep ownership accountable, app-channel sessions adopt the App row's owner instead of the caller's:

- `session.owner_principal_id = app.owner_principal_id`
- `session.resolved_owner_user_id = app.resolved_owner_user_id`

This invariant has security and routing consequences:

- `find_app_session_by_tags_and_owner` keys the lookup on `org_id` + `app_id` + `owner_principal_id` and requires the candidate session's `tags` to **contain** every routing tag passed in (Postgres `tags @> $tags`, not strict equality). Reuse therefore never crosses orgs, never hops apps, and never adopts a session owned by a different principal, even if the surface tags overlap. App-channel sessions also carry the internal `__internal:app_invocation` routing tag so unrelated user sessions cannot satisfy the lookup.
- The internal-only tag prefixes — `__internal:`, `app:`, `app_channel:`, `slack:app:`, and `ag_ui:app:` — are reserved at session create/update time. `SessionService::create` and `update` reject them from non-internal callers (see the matching error in `crates/server/src/domains/sessions/service.rs`), so an org member cannot pre-seed a personal session that an app-channel invocation would later adopt or attach to a sibling app's budget.
- Per-invocation sessions inherit the same owner override even though they are not subject to reuse — this keeps audit, budgets, and policy evaluation consistent across both `InvocationSessionMode` values.

Cross-references:

- `SessionService::create_from_app` in `crates/server/src/domains/sessions/service.rs` for the override implementation.
- `specs/threat-model.md` TM-AUTHZ-006 (anonymous webhook reaching draft/disabled channels), TM-AUTHZ-009 (tag spoofing into app budgets), and TM-A2A-007 (cross-org session reuse via tag spoofing).

## Message Templates

Both invocation channels store a required `message` string. It is rendered at trigger time with `{{path.to.value}}` interpolation.

Common template context:

- `app.id`, `app.name`
- `channel.id`, `channel.type`
- `invocation.source`
- `invocation.triggered_at`

Schedule-only context:

- `schedule.cron_expression`
- `schedule.timezone`

Webhook-only context:

- `payload`
- `webhook.body`
- `webhook.json`
- `webhook.headers`

## Schedule Channel

Config fields:

- `cron_expression`
- `timezone`
- `session_mode`
- `message`

`cron_expression` accepts standard 5-field cron input and the durable
scheduler's 7-field format. App channel creation/update normalizes 5-field
input to the canonical 7-field durable expression before storing the channel
config or syncing the backing durable schedule.

Behavior:

- accepted cron input is either 5-field (`*/10 * * * *`) or 7-field
  (`0 */10 * * * * *`); 5-field input is normalized to the durable
  scheduler's 7-field representation when stored
- create/update/delete stays in the app domain
- the app domain creates or updates the backing durable schedule
- publish state and channel `enabled` control whether the durable binding is enabled
- deleting the app channel deletes the durable binding
- unpublishing disables the binding without deleting it
- `POST /v1/apps/{app_id}/channels/{channel_id}/trigger` manually invokes a
  schedule channel for testing without exposing the backing durable schedule ID

The durable scheduler only knows how to fire `invoke_scheduled_app_channel`. All app-specific resolution happens in the app domain at execution time.

Limits:

- Minimum cron interval: 300 seconds (5 minutes) by default; configurable via
  `SCHEDULE_CHANNEL_MIN_INTERVAL_SECONDS`. Create/update rejects expressions
  that would fire more frequently.
- Maximum enabled schedule channels per org: 10 by default; configurable via
  `SCHEDULE_CHANNEL_MAX_PER_ORG`. Enabling a channel (create with
  `enabled=true`, or update from disabled to enabled) is rejected when the
  org is already at the cap.

## Webhook Channel

Config fields:

- `token`
- `session_mode`
- `message`

Ingress:

- `POST /v1/apps/{app_id}/webhooks/{channel_id}`
- auth via `Authorization: Bearer <token>` or `X-Everruns-Webhook-Token: <token>`
- future webhook auth implementations should use the same
  `channel_config.auth` verifier as AG-UI and A2A instead of adding
  channel-local credential logic. Until that enforcement lands, webhook
  channel configs must not include `auth`.

Behavior:

- only published apps with enabled webhook channels accept requests
- request payload and headers become template context
- the trigger injects the rendered message into the selected session routing mode

## Platform Surfaces

Invocation channels must be reachable through all app-management surfaces:

- HTTP app APIs
- MCP/bash command catalog (`create_app`, `list_app_channels`,
  `add_app_channel`, `trigger_app_schedule_channel`, etc.). Generic durable
  schedule commands are intentionally not exposed through MCP scripting; app
  schedule channels own their lifecycle.
- `platform_management` capability (`read_apps`, `manage_apps`, `manage_app_channels`)
- Apps UI

Secrets in channel configs are write-only in user-facing responses. App and
channel reads return only non-secret fields plus `*_configured` booleans where
callers need to know whether a secret exists.

## Testing

Coverage should include:

1. durable schedule binding lifecycle across create, publish, unpublish, update, disable, delete
2. shared-session and per-invocation behavior for both channel types
3. webhook authentication failures
4. template rendering with structured payloads and raw bodies
5. MCP/bash command execution for app/channel operations
6. platform capability coverage for app/channel management
