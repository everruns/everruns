# Slack Bot Integration

## Abstract

Slack integration allows deploying agents as Slack bots. An App binds an agent and harness to a Slack workspace with per-app signing secret verification and configurable session strategies.

## Architecture

```
Slack Events API  -->  POST /v1/apps/{app_id}/slack/events  (unauthenticated)
                       |
                       +-- Verify HMAC-SHA256 signing secret
                       +-- Find/create session (by tags, per session_strategy)
                       +-- Create user message (triggers agent workflow)
                       +-- Background: poll for output.message.completed
                       +-- Post response to Slack via chat.postMessage
```

## Design Decisions

- **App-scoped endpoint**: Slack is bound to an App, so the webhook is `POST /v1/apps/{app_id}/slack/events`. The App defines the agent, harness, signing secret, and session strategy.
- **Unauthenticated**: Webhook requests come from Slack, not our users. Security is via Slack signing secret verification (HMAC-SHA256), not API key auth.
- **Unscoped app lookup**: `get_app_by_public_id_unscoped()` looks up apps by public_id across all orgs since there's no auth context on webhook requests.
- **Session routing via tags**: Sessions are found/created using tags like `slack:thread:{ts}`, `slack:channel:{id}`, or `slack:user:{id}` depending on the session strategy.
- **Async response delivery**: The webhook acks Slack immediately (<3s), then a background task polls for agent output events and posts the response via Slack's `chat.postMessage` API.

## Channel Config

See `crates/core/src/app.rs` for `SlackChannelConfig` struct.

Key fields:
- `signing_secret`: Slack app signing secret for HMAC-SHA256 verification
- `bot_token`: Slack Bot OAuth token (`xoxb-...`) for sending responses
- `channel_id`: Optional channel to listen on
- `team_id`: Optional Slack workspace ID
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
| POST | `/v1/apps/{app_id}/slack/events` | Slack signing secret | Slack Events API webhook |

All other App CRUD endpoints remain under standard API key auth at `/v1/apps`.

## UI

Apps page at `/apps` with:
- List of apps with status badges
- Create/Edit dialog with Slack configuration fields
- Publish/Unpublish actions
- Webhook URL display (for copying into Slack app config)
- Delete confirmation

## Files

- `crates/core/src/app.rs` - `SlackChannelConfig`, `SessionStrategy` types
- `crates/server/src/api/slack_events.rs` - Webhook endpoint, signing verification, session routing
- `crates/server/src/services/app.rs` - `get_by_public_id_unscoped()` method
- `apps/ui/src/app/(main)/apps/page.tsx` - Apps UI page
- `apps/ui/src/lib/api/apps.ts` - App API functions
- `apps/ui/src/hooks/use-apps.ts` - React Query hooks
