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
                              +-- Register with SlackDeliveryDispatcher (event-driven)
                              +-- Dispatcher: subscribe to EventNotificationBroadcaster
                              +-- On output.message.completed → post to Slack (with retry)
                              +-- On turn.completed / turn.failed → unregister
```

The events endpoint verifies HMAC-SHA256 signing secret, finds/creates session by tags, creates user message (triggers agent workflow), and registers with the `SlackDeliveryDispatcher` for event-driven response delivery.

## Design Decisions

- **One Slack App per Everruns App**: Each app has its own identity (name, avatar, scopes). This is unlike GitHub (global app) because Slack bots are user-facing with distinct identities per use case.
- **Per-app manifest generation**: The manifest endpoint generates a YAML with correct scopes and bot user. `event_subscriptions` is omitted (requires live webhook URL — must be configured after publishing).
- **App-scoped endpoint**: Slack is bound to an App, so the webhook is `POST /v1/apps/{app_id}/slack/events`. The App defines the agent, harness, signing secret, and session strategy.
- **Unauthenticated**: Webhook and manifest requests come from Slack or the browser. Security is via Slack signing secret verification (HMAC-SHA256), not API key auth.
- **Unscoped app lookup**: `get_app_by_public_id_unscoped()` looks up apps across all orgs since webhooks have no auth context.
- **Session routing via tags**: Sessions are found/created using tags like `slack:thread:{ts}`, `slack:channel:{id}`, or `slack:user:{id}` depending on the session strategy.
- **Event-driven response delivery**: The webhook acks Slack immediately (<3s), then registers with `SlackDeliveryDispatcher`. The dispatcher subscribes to `EventNotificationBroadcaster` (PostgreSQL NOTIFY) and delivers `output.message.completed` text to Slack as events arrive, with no fixed deadline. Handles arbitrarily long agent turns. Posts are retried with exponential backoff (3 attempts) on transient failures. Non-retryable errors (invalid token, channel not found) fail immediately. The dispatcher unregisters on `turn.completed` or `turn.failed`. Events are filtered by `input_message_id` to avoid cross-turn interference. Falls back to legacy 120s polling in DEV_MODE (no PostgreSQL).
- **Startup recovery**: On server restart, `SlackDeliveryDispatcher::recover()` queries sessions with `status = 'active'` and `slack:*` tags, looks up the corresponding app for the bot_token, finds the last unfinished turn, and re-registers deliveries.
- **Slack event dedup**: Slack sends both `app_mention` and `message` events for @mentions. DB-level dedup via `has_event_with_slack_ts()` prevents duplicate processing (uses JSONB `@>` containment on input.message events).
- **Thread context injection**: When the bot is first mentioned mid-thread (`PerThread` strategy, new session, `thread_ts` present), prior messages are fetched via Slack's `conversations.replies` API and injected as `input.message` events (without triggering agent workflows). This gives the agent full conversational context. Bot messages become assistant-role; human messages get user-role with `ExternalActor` attribution. Failures are non-fatal — the agent proceeds without history. Required scopes (`channels:history`, `groups:history`, `im:history`, `mpim:history`) are already in the manifest.
- **Reply modes**: Slack apps can either forward completed assistant messages (`all_messages`) or run in `report_progress_only` handoff mode. In handoff mode the webhook posts an immediate deterministic acknowledgement (`On it.`), the session is tagged with the reply mode, ReasonAtom exposes a `report_progress` tool + prompt instructions, and Slack delivery ignores normal assistant messages in favor of explicit `tool.completed` events from that tool.

## Channel Config

See `crates/core/src/app.rs` for `SlackChannelConfig` struct.

Key fields:
- `signing_secret`: Slack app signing secret for HMAC-SHA256 verification
- `bot_token`: Slack Bot OAuth token (`xoxb-...`) for sending responses
- `channel_id`: Optional channel to listen on
- `team_id`: Slack workspace ID
- `session_strategy`: `per_thread` (default), `per_channel`, `per_user`
- `reply_mode`: `all_messages` (default) or `report_progress_only`

## Session Strategies

| Strategy | Tag Pattern | Behavior |
|----------|-------------|----------|
| `per_thread` | `slack:thread:{thread_ts}` | Each Slack thread = separate session |
| `per_channel` | `slack:channel:{channel}` | One session per channel |
| `per_user` | `slack:user:{user}` | One session per user |

## Reply Modes

| Mode | Slack-visible behavior |
|------|------------------------|
| `all_messages` | Every completed assistant message is posted back to Slack |
| `report_progress_only` | Slack gets `On it.` immediately, then only deterministic `report_progress` updates |

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

## Message Attachments

Slack messages can include file uploads (`files[]`) and legacy attachments (`attachments[]` — link unfurls, Canvas, Workflows, etc.). Both are processed into `InputContentPart` items appended after the text content:

- **Image files** (png, jpeg, gif, webp) with a `url_private` → `InputContentPart::Image(url)` (requires `files:read` scope)
- **Non-image files** (pdf, video, text snippets, etc.) → text description: `[Attached file: name (type)]`
- **Legacy attachments with `image_url`** → `InputContentPart::Image(url)`
- **Legacy attachments without image** → text summary from title/text/service_name/fallback fields
- **Empty attachments** (no usable fields) → skipped

Messages with only attachments (no text) are not dropped.

Future: a Slack file fetch tool could let agents pull file content on demand via `url_private` + bot token auth.

## User Identity

Slack user identity is carried via the channel-agnostic `ExternalActor` struct (see `crates/core/src/message.rs`). The Slack handler:

1. Resolves Slack user ID → display name via `users.info` API (requires `users:read` scope)
2. Populates `ExternalActor { actor_id, actor_name, source: "slack", metadata }` on the message
3. The `ReasonAtom` generically prefixes user messages with `[display_label]` so the LLM knows who is speaking

Display name resolution uses an in-memory cache with:
- Successful lookups cached permanently (per instance lifetime)
- Permanent API errors (`missing_scope`, `invalid_auth`, etc.) cached as `None` to avoid repeated calls
- Transient network errors not cached (allow retry)

The API `Message` response includes `external_actor` (optional) so clients can display user identity for externally-originated messages.

This is channel-agnostic — any future channel adapter (Discord, Teams) populates the same `ExternalActor` struct and gets the same LLM prefix behavior.

## Testing

Integration tests in `crates/server/tests/slack_integration_test.rs`:

- **Webhook tests** (always run): URL verification, signature rejection, session creation/reuse, bot message filtering, session strategies, manifest endpoint, replay attack prevention
- **Real Slack API tests** (require credentials): `chat.postMessage`, `users.info`, full webhook→session flow with real signing secret

CI runs all tests via `doppler run` in the `integration-test` job (PostgreSQL required). `real_slack_credentials()` always panics if any `TEST_SLACK_*` env var is missing — real-API tests never silently skip.

Doppler vars: `TEST_SLACK_BOT_TOKEN`, `TEST_SLACK_SIGNING_SECRET`, `TEST_SLACK_TEST_CHANNEL`.

## Files

- `crates/core/src/app.rs` - `SlackChannelConfig`, `SessionStrategy` types
- `crates/core/src/message.rs` - `ExternalActor` struct
- `crates/server/src/api/messages.rs` - API `Message` response includes `external_actor`
- `crates/server/src/api/slack_events.rs` - Webhook endpoint, manifest generation, signing verification, session routing, user name resolution
- `crates/server/src/slack_delivery.rs` - Event-driven Slack delivery dispatcher with retry and startup recovery
- `crates/server/src/services/app.rs` - `get_by_public_id_unscoped()` method
- `apps/ui/src/app/(main)/apps/page.tsx` - Apps list UI page
- `apps/ui/src/app/(main)/apps/new/page.tsx` - App creation page
- `apps/ui/src/app/(main)/apps/[appId]/page.tsx` - App detail page with manifest + config
- `apps/ui/src/lib/api/apps.ts` - App API functions including `getSlackManifest()`
- `apps/ui/src/hooks/use-apps.ts` - React Query hooks
- `docs/integrations/slack.md` - User-facing setup guide
