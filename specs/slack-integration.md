# Slack Bot Integration

## Abstract

Slack integration allows deploying agents as Slack bots. Each Everruns App gets its own Slack App (own identity, name, avatar). An App binds an agent and harness to a Slack workspace with signing secret verification and configurable session strategies. Setup is streamlined via per-app manifest generation.

## Architecture

```
Per-app manifest (recommended):
  UI "Create Slack App"  -->  GET /v1/apps/{app_id}/slack/manifest  (returns YAML + create URL)
                              |
  Opens Slack "Create from manifest" with pre-filled scopes + bot user
                              |
  User copies signing_secret + bot_token back to Everruns

Slack Events API         -->  POST /v1/apps/{app_id}/slack/events   (per-app, uses per-app secret)
                              |
                              +-- Verify HMAC-SHA256 signing secret
                              +-- Find/create session (by tags, per session_strategy)
                              +-- Create user message (triggers agent workflow)
                              +-- Dedup: skip if slack_ts already processed (DB check)
                              +-- Background: poll events, stream each output.message.completed
                              +-- Post each response to Slack via chat.postMessage
                              +-- Stop on turn.completed / turn.failed
```

The events endpoint verifies HMAC-SHA256 signing secret, finds/creates session by tags, creates user message (triggers agent workflow), polls for `output.message.completed`, and posts response to Slack via `chat.postMessage`.

## Design Decisions

- **One Slack App per Everruns App**: Each app has its own identity (name, avatar, scopes). This is unlike GitHub (global app) because Slack bots are user-facing with distinct identities per use case.
- **Per-app manifest generation**: The manifest endpoint generates a YAML with correct scopes and bot user. `event_subscriptions` is omitted (requires live webhook URL — must be configured after publishing).
- **App-scoped endpoint**: Slack is bound to an App, so the webhook is `POST /v1/apps/{app_id}/slack/events`. The App defines the agent, harness, signing secret, and session strategy.
- **Unauthenticated**: Webhook and manifest requests come from Slack or the browser. Security is via Slack signing secret verification (HMAC-SHA256), not API key auth.
- **Unscoped app lookup**: `get_app_by_public_id_unscoped()` looks up apps across all orgs since webhooks have no auth context.
- **Session routing via tags**: Sessions are found/created using tags like `slack:thread:{ts}`, `slack:channel:{id}`, or `slack:user:{id}` depending on the session strategy.
- **Streaming response delivery**: The webhook acks Slack immediately (<3s), then a background task polls for agent output events. Each `output.message.completed` with text is posted to Slack as it arrives, giving users real-time progress during multi-step turns. The poller stops on `turn.completed` or `turn.failed`. Events are filtered by `input_message_id` to avoid cross-turn interference.
- **Slack event dedup**: Slack sends both `app_mention` and `message` events for @mentions. DB-level dedup via `has_event_with_slack_ts()` prevents duplicate processing (uses JSONB `@>` containment on input.message events).

## Channel Config

See `crates/core/src/app.rs` for `SlackChannelConfig` struct.

Key fields:
- `signing_secret`: Slack app signing secret for HMAC-SHA256 verification
- `bot_token`: Slack Bot OAuth token (`xoxb-...`) for sending responses
- `channel_id`: Optional channel to listen on
- `team_id`: Slack workspace ID
- `session_strategy`: `per_thread` (default), `per_channel`, `per_user`

## Session Strategies

| Strategy | Tag Pattern | Behavior |
|----------|-------------|----------|
| `per_thread` | `slack:thread:{thread_ts}` | Each Slack thread = separate session |
| `per_channel` | `slack:channel:{channel}` | One session per channel |
| `per_user` | `slack:user:{user}` | One session per user |

## API

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/v1/apps/{app_id}/slack/events` | Slack signing secret | Per-app Slack Events API webhook |
| GET | `/v1/apps/{app_id}/slack/manifest` | None | Returns Slack App manifest YAML + create URL |

All other App CRUD endpoints remain under standard API key auth at `/v1/apps`.

## UI

Apps page at `/apps` with:
- List of apps with status badges
- Create page at `/apps/new` with name, harness, agent selection
- Detail page at `/apps/{id}` with:
  - "Create Slack App" button (opens Slack with pre-filled manifest)
  - Manual configuration fields (signing secret, bot token)
  - Webhook URL display for Event Subscriptions
- Publish/Unpublish actions
- Delete confirmation

## User Identity

Slack user identity is carried via the channel-agnostic `ExternalActor` struct (see `crates/core/src/message.rs`). The Slack handler:

1. Resolves Slack user ID → display name via `users.info` API (requires `users:read` scope)
2. Populates `ExternalActor { actor_id, actor_name, source: "slack", metadata }` on the message
3. The `ReasonAtom` generically prefixes user messages with `[display_label]` so the LLM knows who is speaking

Display name resolution uses an in-memory cache with:
- Successful lookups cached permanently (per instance lifetime)
- Permanent API errors (`missing_scope`, `invalid_auth`, etc.) cached as `None` to avoid repeated calls
- Transient network errors not cached (allow retry)

This is channel-agnostic — any future channel adapter (Discord, Teams) populates the same `ExternalActor` struct and gets the same LLM prefix behavior.

## Files

- `crates/core/src/app.rs` - `SlackChannelConfig`, `SessionStrategy` types
- `crates/core/src/message.rs` - `ExternalActor` struct
- `crates/server/src/api/slack_events.rs` - Webhook endpoint, manifest generation, signing verification, session routing, user name resolution
- `crates/server/src/services/app.rs` - `get_by_public_id_unscoped()` method
- `apps/ui/src/app/(main)/apps/page.tsx` - Apps list UI page
- `apps/ui/src/app/(main)/apps/new/page.tsx` - App creation page
- `apps/ui/src/app/(main)/apps/[appId]/page.tsx` - App detail page with manifest + config
- `apps/ui/src/lib/api/apps.ts` - App API functions including `getSlackManifest()`
- `apps/ui/src/hooks/use-apps.ts` - React Query hooks
- `docs/integrations/slack.md` - User-facing setup guide
