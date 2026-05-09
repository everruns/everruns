# App Invocation Channels

## Abstract

App invocation channels are unattended ingress modes on top of the App abstraction. They let an App inject a configured user message into an app-owned session when an external trigger fires.

Current invocation channels:

- `schedule` — app-owned durable cron trigger
- `webhook` — authenticated HTTP trigger

This spec is intentionally separate from:

- session-local scheduling concerns
- `specs/scheduled-tasks.md`, which defines the generic durable scheduler

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

- create/update/delete stays in the app domain
- the app domain creates or updates the backing durable schedule
- publish state and channel `enabled` control whether the durable binding is enabled
- deleting the app channel deletes the durable binding
- unpublishing disables the binding without deleting it

The durable scheduler only knows how to fire `invoke_scheduled_app_channel`. All app-specific resolution happens in the app domain at execution time.

## Webhook Channel

Config fields:

- `token`
- `session_mode`
- `message`

Ingress:

- `POST /v1/apps/{app_id}/webhooks/{channel_id}`
- auth via `Authorization: Bearer <token>` or `X-Everruns-Webhook-Token: <token>`

Behavior:

- only published apps with enabled webhook channels accept requests
- request payload and headers become template context
- the trigger injects the rendered message into the selected session routing mode

## Platform Surfaces

Invocation channels must be reachable through all app-management surfaces:

- HTTP app APIs
- MCP/bash command catalog (`create_app`, `add_app_channel`, `update_app_channel`, etc.)
- `platform_management` capability (`read_apps`, `manage_apps`, `manage_app_channels`)
- Apps UI

## Testing

Coverage should include:

1. durable schedule binding lifecycle across create, publish, unpublish, update, disable, delete
2. shared-session and per-invocation behavior for both channel types
3. webhook authentication failures
4. template rendering with structured payloads and raw bodies
5. MCP/bash command execution for app/channel operations
6. platform capability coverage for app/channel management
