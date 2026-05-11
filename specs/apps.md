# Apps

## Abstract

An App is a deployable unit that binds a Harness and optional Agent to one or more invocation channels. Interactive channels such as Slack and AG-UI accept user traffic directly. Invocation channels such as `schedule` and `webhook` inject a configured message into an app-owned session when an external trigger fires. The publish/unpublish lifecycle controls whether those channels actively accept or emit new invocations.

See [app-invocation-channels.md](app-invocation-channels.md) for the dedicated spec covering schedule/webhook behavior, session routing, templates, and durable bindings.

Authentication for App-published endpoints (AG-UI, A2A, webhook, and future channels) is described by the shared framework in [app-endpoint-auth.md](app-endpoint-auth.md). Channels keep their existing legacy auth fields for backward compatibility and adopt the shared model when an explicit policy is configured.

## Concepts

### App

Top-level deployment entity. Composes existing building blocks (Harness, optional Agent) with one or more distribution channels.

- Each App references exactly one Harness (required)
- Each App references zero or one Agent
- When `FEATURE_AGENT_VERSIONS` is enabled, each App may choose an Agent version policy: `default`, `latest`, or `pinned`. See [agent-versions.md](agent-versions.md).
- Each App has zero or more **Channels** (stored in `app_channels` table)
- Apps have a publish lifecycle: `draft` → `published` → `draft`
- Apps also participate in the default building-block lifecycle: `active/draft/published -> archived -> deleted`
- Only published apps accept incoming requests

### App Channel

A distribution channel attached to an App. Each channel has its own type, config, and enabled flag.

- Stored in `app_channels` table (one-to-many with `apps`)
- Uses dual-ID pattern: `appchan_` prefix
- `channel_type` is optional at create time; when present it creates the first channel automatically
- Channels can be added, updated, or removed via `/v1/apps/{app_id}/channels`
- Schedule channels additionally own an internal durable schedule binding (`app_channels.durable_schedule_id`) used to synchronize app lifecycle with the durable scheduler

### Channel Types

Current: `slack`, `ag_ui`, `schedule`, `webhook`. Future: `whatsapp`, `web_widget`, `api_endpoint`, `discord`, etc.

Channel config is stored as JSONB and validated at the application layer per channel type.

**Slack channel config example:**
```json
{
  "channel_id": "C0123456789",
  "channel_name": "#support",
  "team_id": "T0123456789",
  "session_strategy": "per_thread",
  "reply_mode": "all_messages"
}
```

**AG-UI channel config example:**
```json
{
  "anonymous": true,
  "token": "optional-shared-secret",
  "session_expiration_seconds": 21600,
  "rate_limit_per_minute": 60,
  "tool_visibility": "generic",
  "generic_tool_text": "Working..."
}
```

AG-UI uses an app-scoped anonymous ingress for the initial rollout:

- Endpoint: `POST /v1/apps/{app_id}/ag-ui`
- Requests use AG-UI `RunAgentInput` JSON
- Responses stream back as AG-UI SSE events translated from the durable runtime
- Images may be uploaded through `POST /v1/apps/{app_id}/ag-ui/images`
  as multipart form field `file`. The route uses the same published-app,
  anonymous, token, and rate-limit gates as the stream endpoint. The returned
  image IDs may be passed on a run in `forwardedProps.imageIds`; the handler
  attaches only images uploaded through the same app's public AG-UI route.
- Anonymous access is currently required when the channel is enabled
- If `token` is set, requests must include either `Authorization: Bearer <token>` or `X-Everruns-AG-UI-Token: <token>`.
- Session routing is per `threadId`, with sessions tagged by app and thread
- Request body validation (see threat `TM-LLM-020`):
  - `messages[*].role` must be `user` or `assistant`. `system`, `developer`, and `tool` are rejected with `400 invalid_request` and the offending role is not echoed back.
  - `messages[*].id` must be a valid UUID (enforced by `MessageId` deserialization) and unique within the request; duplicates are rejected with `400 invalid_request`.
  - The CopilotKit single-route runtime forwards into this same endpoint, so its anonymous consumers inherit these gates without per-deployment work.
- `session_expiration_seconds` caps how long a `threadId` can resume the
  underlying session. After the window elapses, AG-UI requests with that
  `threadId` are rejected with `410 Gone` and the client must start a new
  thread. Defaults to `21600` (6 hours). Set to `0` to disable expiration.
- `rate_limit_per_minute` is an optional per-IP, per-app cap. `0` or absent disables the per-app cap and only the global API limit applies. Values above 1,000,000 are rejected at write time. Counters are shared across instances when `VALKEY_URL` is set; otherwise enforcement is per-instance.
- Public tool activity is controlled by `tool_visibility`: `none` suppresses tool activity entirely, `generic` emits only `generic_tool_text` as transient activity, and `narrated` emits backend-authored narration. Public AG-UI streams never expose raw tool names, arguments, results, or internal tool call IDs.

`schedule` and `webhook` are app invocation channels, not interactive messaging adapters:

- `schedule` owns a managed durable schedule binding
- `webhook` exposes a token-authenticated app-scoped HTTP endpoint
- both inject a configured user message into an app-owned session
- both support `shared_session` and `session_per_invocation`

Examples and detailed behavior live in [app-invocation-channels.md](app-invocation-channels.md).

### Session Strategy

Controls how incoming messages map to sessions:
- `per_thread` — each thread gets its own session (default)
- `per_channel` — one session per channel
- `per_user` — one session per user

### Slack Reply Mode

Controls what gets posted back to Slack:
- `all_messages` — forward completed assistant messages directly to Slack (default)
- `report_progress_only` — send an immediate handoff acknowledgement, then only forward explicit `report_progress` tool updates

### Lifecycle

```
draft → published → draft → archived → deleted
         ↕
    (accepting requests)
```

- `draft`: App is configured but not accepting requests
- `published`: App is live, interactive channels accept traffic and automation channels may create/continue sessions
- Unpublishing stops new message processing; existing sessions remain
- `archived`: Read-only, hidden from lists by default, not assignable, not executable
- `deleted`: Tombstone state for historical references only; normal detail API returns `404`

## UI

The App detail page has two implementations during the redesign rollout:

- `apps.detailV2=false`: legacy configuration/detail page.
- `apps.detailV2=true`: channels-first operations page.

The channels-first page folds App configuration into the header, renders a Health / Invocations 24h / Success rate / Activity stat strip, lists channels as expandable rows, and shows a live activity rail. Channel creation and editing are full-page routes, not dialogs:

- `/apps/{app_id}/channels/new`
- `/apps/{app_id}/channels/{channel_id}`

Schedule labels in rows, headers, breadcrumbs, summaries, and other read-only UI must render a human-readable cron description plus timezone. Raw cron expressions are only shown inside the editable Cron input.

## Data Model

See `crates/core/src/app.rs` for the complete `App` and `AppChannel` definitions.

Version binding fields:

- `agent_version_policy`: `default`, `latest`, or `pinned`
- `agent_version_id`: required only when policy is `pinned`

## API

All endpoints under `/v1/apps`. See `crates/server/src/api/apps.rs`.

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/apps` | Create app |
| GET | `/v1/apps` | List non-archived apps by default (`include_archived=true` to include archived) |
| GET | `/v1/apps/{app_id}` | Get app |
| PATCH | `/v1/apps/{app_id}` | Update app |
| DELETE | `/v1/apps/{app_id}` | Archive app |
| POST | `/v1/apps/{app_id}/delete` | Dangerous delete of archived app |
| POST | `/v1/apps/{app_id}/publish` | Publish app |
| POST | `/v1/apps/{app_id}/unpublish` | Unpublish app |
| POST | `/v1/apps/{app_id}/channels` | Add a channel |
| PATCH | `/v1/apps/{app_id}/channels/{channel_id}` | Update a channel |
| DELETE | `/v1/apps/{app_id}/channels/{channel_id}` | Remove a channel |
| POST | `/v1/apps/{app_id}/channels/{channel_id}/trigger` | Manually trigger a schedule channel |
| POST | `/v1/apps/{app_id}/webhooks/{channel_id}` | Trigger a webhook channel |

## ID Schema

| Entity | Prefix | Example |
|--------|--------|---------|
| App | `app_` | `app_01933b5a00007000800000000000001` |
| App Channel | `appchan_` | `appchan_01933b5a00007000800000000000002` |
