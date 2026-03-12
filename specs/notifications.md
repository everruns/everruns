# Notifications

## Intent

Provide a generic, durable notification system for user-facing delivery surfaces:

- UI bell with counter
- UI toast surface
- Future email / external channels

Notifications are canonical server records. UI surfaces are projections of the same data, not separate systems.

## Scope

Initial notification type:

- `turn.long_running_completed`
  - Emitted when a turn completes after at least 60 seconds of work
  - Target links back to the chat UI for that session

## Model

Notifications are user-scoped and org-scoped.

Core fields:

- `id`
- `org_id`
- `user_id`
- `kind`
- `title`
- `body`
- optional target metadata: `target_type`, `target_id`, `href`
- arbitrary `payload`
- `occurrence_count`
- `viewed_at`
- `created_at`, `updated_at`

Design notes:

- `viewed_at` drives the bell counter
- `occurrence_count` supports dedupe without spamming users
- `href` is optional so future channels are not forced to be URL-based

See `crates/server/src/api/notifications.rs` and `crates/server/src/storage/models.rs` for the concrete API and persistence shapes.

## Creation Flow

Long-running turn notifications resolve the recipient from the input message that started the turn:

1. User sends a message
2. Server stores `input_message_id -> (org_id, user_id, session_id)`
3. `turn.completed` listener checks `duration_ms`
4. If duration is at least 60 seconds, server creates a notification for that user

## Delivery Surfaces

### Bell

- Uses durable notification records
- Counter shows unviewed notifications
- Opening the bell does not auto-view items
- Clicking an item marks it viewed

### Toast

- Secondary surface for foreground sessions
- Only for newly-arrived notifications while the app is visible/focused
- Never the source of truth

## Active Chat Suppression

V1 suppression is client-side:

- If the user is actively viewing `/sessions/{session_id}/chat`
- and the tab is visible
- and the window is focused

then matching notifications are filtered before rendering:

- no toast
- no bell increment in effective UI state
- client immediately marks the notification viewed in the background

This avoids flicker while keeping the server model simple.

## Abuse Controls

- Long-running turn notifications are only emitted for the requesting user
- Dedupe uses `dedupe_key`
- Repeated matches bump `occurrence_count` instead of inserting duplicates
- Per-kind unviewed count is capped before new notifications are created

## Transport

- REST bootstrap for initial notification list and unviewed count
- SSE for incremental `notification.upsert` delivery
- PostgreSQL `LISTEN/NOTIFY` wakes streams in production
- DEV mode falls back to timeout-based polling
