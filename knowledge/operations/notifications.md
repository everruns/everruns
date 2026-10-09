---
type: Specification
title: "Notifications"
description: "Generic user notifications."
tags:
  - everruns
  - operations
---
# Notifications

## Intent

Provide a generic, durable notification system for user-facing delivery surfaces:

- UI bell with counter
- UI toast surface
- Future email / external channels

Notifications are canonical server records. UI surfaces are projections of the same data, not separate systems.

For persistent unresolved operational conditions and guided recovery, see
[Actionable Health Issues](health-issues.md). That contract separates condition state
from per-user notification viewing; its detector and recovery contract live there.

## Scope

Notification kinds:

- `turn.long_running_completed` and `turn.long_running_failed`
  - Emitted when a turn in the recipient's own chat ends after at least 60
    seconds: a session with `source = chat` whose effective owner is the user
    who sent the message. Turns in API, channel, playground, or another
    user's sessions are not something this user is waiting on, so they raise
    nothing.
  - Title is the chat's name (its title, else "Chat with {agent}"); body says
    how it ended and how long it took, followed by the start of the answer.
  - Target opens the chat (`/chats/{session_id}`), not the session inspector.
- `health.issue`, see [Actionable Health Issues](health-issues.md).

Every kind shares one shape, so a new producer (a shared agent posting to its
users, an integration) adds a kind string and a creator, not a client change:
the title names the thing, the body says what happened in plain text, the
source names the sender, and the target says what opens.

## Model

Notifications are user-scoped and org-scoped.

Core fields:

- `id`
- `org_id`
- `user_id`
- `kind`
- `title`
- `body`
- optional `source`: who sent it, `type` (`agent` or `system`), `id`, `name`
- optional target metadata: `target_type`, `target_id`, `href`
- arbitrary `payload`
- `occurrence_count`
- `viewed_at`
- `created_at`, `updated_at`

Design notes:

- `viewed_at` drives the bell counter
- `occurrence_count` supports dedupe without spamming users
- `href` is optional so future channels are not forced to be URL-based

See `crates/server/src/api/notifications.rs` and `crates/server/src/storage/models/mod.rs` for the concrete API and persistence shapes.

## Creation Flow

Long-running turn notifications resolve the recipient from the input message that started the turn:

1. User sends a message
2. Server stores `input_message_id -> (org_id, user_id, session_id)`
3. The `turn.completed` listener reads `duration_ms`; the `turn.failed`
   listener, which has no duration, measures from when the message was stored
4. If the turn ran at least 60 seconds and the session is that user's own chat,
   server creates a notification for that user

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

## Feature Flag

- Gated by standard flag `FEATURE_NOTIFICATIONS`
- Default: off in all environments
- When disabled:
  - Notification API routes are not mounted
  - Turn-completed notification creation is disabled
  - UI bell, toast, and notification SSE are not mounted

## Active Chat Suppression

V1 suppression is client-side:

- If the user is actively viewing the chat (`/chats/{session_id}` or
  `/sessions/{session_id}/chat`)
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

Deployment constraint:

- notification SSE wakeups use a dedicated PostgreSQL listener connection
- if the deployment pools `DATABASE_URL`, operators should provide `DATABASE_UNPOOLED_URL` for notification listener traffic

Reasoning:

- notification wakeups are session-scoped PostgreSQL listener traffic
- sharing them with pooled query sessions can produce intermittent protocol-level failures that are hard to attribute from the notification surface alone
