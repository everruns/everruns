# Apps

## Abstract

An App is a deployable unit that binds a Harness and Agent to a distribution channel (Slack, WhatsApp, web widget, etc.). It provides a publish/unpublish lifecycle that controls whether the app actively accepts incoming requests from its configured channel.

## Concepts

### App

Top-level deployment entity. Composes existing building blocks (Harness, Agent) with channel-specific configuration.

- Each App references exactly one Harness (required)
- Each App references exactly one Agent (required)
- Each App has a channel type and channel-specific config (JSON)
- Apps have a publish lifecycle: `draft` → `published` → `draft`
- Apps also participate in the default building-block lifecycle: `active/draft/published -> archived -> deleted`
- Only published apps accept incoming requests

### Channel Types

Initially: `slack`. Future: `whatsapp`, `web_widget`, `api_endpoint`, etc.

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
- `published`: App is live, incoming messages create/continue sessions
- Unpublishing stops new message processing; existing sessions remain
- `archived`: Read-only, hidden from lists by default, not assignable, not executable
- `deleted`: Tombstone state for historical references only; normal detail API returns `404`

## Data Model

See `crates/core/src/app.rs` for full struct definition.

Key fields:
- `id` / `public_id`: Dual-ID pattern (see `specs/id-schema.md`)
- `name`: Display name
- `description`: Optional
- `harness_id`: Required FK to harness
- `agent_id`: Required FK to agent
- `channel_type`: Enum string (`slack`, etc.)
- `channel_config`: JSONB with channel-specific settings
- Slack `channel_config.reply_mode`: `all_messages` or `report_progress_only`
- `status`: `draft` | `published` | `archived` | `deleted`
- `archived_at`, `deleted_at`: Lifecycle timestamps
- `published_at`: Timestamp when last published
- `created_at`, `updated_at`: Standard timestamps

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

## ID Schema

| Entity | Prefix | Example |
|--------|--------|---------|
| App | `app_` | `app_01933b5a00007000800000000000001` |
