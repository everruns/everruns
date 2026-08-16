---
type: Specification
title: "Apps"
description: "Apps system."
tags:
  - everruns
  - integrations
---
# Apps

## Abstract

An App is a deployable unit that binds a Harness and Agent to one or more invocation channels. Interactive channels such as Slack and AG-UI accept user traffic directly. Webhook invocation channels inject a configured message into an app-owned session when an external trigger fires. The publish/unpublish lifecycle controls whether those channels actively accept new invocations.

See [app-invocation-channels.md](app-invocation-channels.md) for webhook behavior and the legacy schedule-channel migration contract.

Authentication for App-published endpoints is described by the shared framework in [app-endpoint-auth.md](app-endpoint-auth.md). AG-UI and A2A keep their existing legacy auth fields for backward compatibility and adopt the shared model when inline `channel_config.auth` is configured.

## Concepts

### App

Top-level deployment entity. Composes existing building blocks (Harness and Agent) with one or more distribution channels.

- Each App references exactly one Harness (required)
- Each newly created or updated App references exactly one Agent
- The storage field remains nullable solely to grandfather existing agent-less Apps. Those rows are not backfilled or rewritten and remain runtime-compatible, but any later App update must assign an Agent.
- When `FEATURE_AGENT_VERSIONS` is enabled, each App may choose an Agent version policy: `default`, `latest`, or `pinned`. See [agent-versions.md](../runtime-resources/agent-versions.md).
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

### Channel Types

Current: `slack`, `ag_ui`, `webhook`, `a2a`, `fcp`, `api_endpoint`, `public_chat`. Future: `whatsapp`, `web_widget`, `discord`, etc. The `api_endpoint` channel carries an app-scoped, execution-only API key over the native session API; see [app-api-keys.md](app-api-keys.md).

The former App `schedule` channel is deprecated. New schedule channels are rejected with guidance to create an Agent trigger. Migration `106_migrate_app_schedules_to_agent_triggers.sql` converts each agent-bound schedule channel to an Agent trigger, preserves the cron/timezone/session mode/message config and existing durable schedule identity, then removes the old channel binding. Agent-less grandfather rows, including any schedule binding they already own, are left unchanged.

`public_chat` is an isolated, public-facing chat web app bound to a single App's agent (anonymous by default, optional Google sign-in, optional Cloudflare Turnstile bot mitigation, plus branding). It reuses AG-UI streaming and the shared App endpoint auth verifier. See [public-chat.md](public-chat.md).

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
  "auth": {
    "mode": "google_oidc",
    "provider": {
      "type": "google_oidc",
      "client_id": "1234567890-abc.apps.googleusercontent.com",
      "allowed_domains": ["example.com"]
    },
    "requirements": {
      "audiences": ["1234567890-abc.apps.googleusercontent.com"]
    }
  },
  "session_expiration_seconds": 21600,
  "rate_limit_per_minute": 60,
  "tool_visibility": "generic",
  "generic_tool_text": "Working...",
  "reasoning_summary_visible": false
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
- If `auth` is set, it replaces the legacy anonymous/token gate with the
  shared App endpoint auth verifier. Supported inline modes are `anonymous`,
  `shared_secret`, `api_key`, `google_oidc`, `oidc`,
  `oauth2_introspection`, `http_basic`, and `mtls`. Google/OIDC and OAuth2
  modes use `Authorization: Bearer <token>`; HTTP Basic uses the standard
  `Authorization: Basic ...` header; mTLS uses a deployment-owned trusted
  reverse-proxy identity header.
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
- Provider reasoning summaries are controlled separately by `reasoning_summary_visible` and default to hidden because anonymous/public apps may run over private prompts, tools, or retrieved data. Even when enabled, opaque `encrypted_content` is never exposed.

### App Endpoint Auth

App-published HTTP endpoints can carry an inline auth config at
`app_channels.channel_config.auth` when their handler is wired into the shared
verifier. The primary product flow is channel-local: create an Agent, create an
App/channel, then configure auth directly on that channel. There is
intentionally no required org-level provider setup in the first iteration. The
runtime types stay provider-shaped so a later UI can add optional reuse actions
such as "save as reusable provider" without changing the endpoint verifier.

Model:

```json
{
  "mode": "oidc",
  "provider": {
    "type": "oidc",
    "issuer": "https://auth.example.com",
    "jwks_url": "https://auth.example.com/.well-known/jwks.json"
  },
  "requirements": {
    "audiences": ["api://support-agent"],
    "scopes": ["invoke:agent"],
    "domains": ["example.com"],
    "groups": ["support"]
  }
}
```

Provider modes:

- `google_oidc` validates Google-issued ID tokens with the configured client
  ID as audience and optional hosted-domain checks.
- `oidc` validates JWT bearer tokens against OIDC discovery/JWKS, requiring
  issuer, audience, expiration, and honoring `nbf`.
- `oauth2_introspection` validates opaque bearer tokens through the configured
  introspection endpoint, then applies the same scope/claim requirements.
- `http_basic` stores only a password hash in channel config responses;
  plaintext passwords are write-only and normalized before storage.
- `mtls` validates a configured trusted identity header set by a reverse proxy
  after client certificate verification. Public edges must strip that header
  from inbound requests before setting it.

Legacy configs remain valid:

- AG-UI without `auth` uses `anonymous` plus optional `token`.
- A2A without `auth` uses the generated per-channel API key.
- Webhook uses its existing token field until its handler is wired into the
  shared verifier. The API rejects webhook `auth` configs so operators do not
  configure a policy that is not enforced.

Auth checks run after published-app/enabled-channel resolution and before any
session lookup, image upload, task polling, cancellation, or message dispatch.
Failures are deliberately generic so callers cannot distinguish provider
misconfiguration from credential probing beyond 401/403 class.

`webhook` is an app invocation channel, not an interactive messaging adapter. It exposes a token-authenticated app-scoped HTTP endpoint, injects a configured user message into an app-owned session, and supports `shared_session` and `session_per_invocation`. Scheduled proactive execution belongs to Agent triggers.

Examples and detailed behavior live in [app-invocation-channels.md](app-invocation-channels.md).

### Session Strategy

Controls how incoming messages map to sessions:
- `per_thread`, each thread gets its own session (default)
- `per_channel`, one session per channel
- `per_user`, one session per user

### Slack Reply Mode

Controls what gets posted back to Slack:
- `all_messages`, forward completed assistant messages directly to Slack (default)
- `report_progress_only`, send an immediate handoff acknowledgement, then only forward explicit `report_progress` tool updates

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

The App detail page is a channels-first operations page. It folds App configuration into the header, renders a Health / Invocations 24h / Success rate / Activity stat strip, lists channels as expandable rows, and shows a live activity rail with an agent-identity control that persists changes inline. Channel creation and editing are full-page routes, not dialogs:

- `/apps/{app_id}/channels/new`
- `/apps/{app_id}/channels/{channel_id}`

Schedule labels in rows, headers, breadcrumbs, summaries, and other read-only UI must render a human-readable cron description plus timezone. Raw cron expressions are only shown inside the editable Cron input.

### Create-app entry points

Apps are the publish surface for Harnesses and Agents. To avoid forcing users to leave the building block they just configured, the Harness and Agent detail pages each expose a **Create app** shortcut (active status only) that links to the App create form with the originating building block prefilled:

- Harness detail → `/apps/new?harness_id={harness_id}`
- Agent detail → `/apps/new?agent_id={agent_id}`

The create form is a streamlined draft form: an **App details** section (name, description) and a **Deployment** section (required Harness and Agent). Channel and agent-identity configuration are deferred to the detail page, so creation always yields a draft. The form reads `harness_id` and `agent_id` from the query string to seed its selectors. Selecting an Agent, from the query-string shortcut or in the form, prefills the harness from that Agent's `harness_id` (still editable).

## Data Model

See `crates/platform/src/app.rs` for the complete `App` and `AppChannel` definitions.

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
| POST | `/v1/apps/{app_id}/webhooks/{channel_id}` | Trigger a webhook channel |

## ID Schema

| Entity | Prefix | Example |
|--------|--------|---------|
| App | `app_` | `app_01933b5a00007000800000000000001` |
| App Channel | `appchan_` | `appchan_01933b5a00007000800000000000002` |
