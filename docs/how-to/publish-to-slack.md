---
title: Publish an agent as a Slack app
description: Bind an agent to a Slack workspace via the Apps system, configure session routing, and publish it so Slack messages reach your agent.
---

An **App** binds a Harness + Agent pair to an external channel and exposes a publish/unpublish lifecycle. This guide deploys an agent as a Slack bot. For the full Slack integration setup (manifest, OAuth, signing secret), see [Slack Integration](/integrations/slack/).

## Prerequisites

- An agent ID (`agent_...`) and harness ID (`harness_...`).
- A Slack app created in the Slack admin console with a signing secret and bot token. See [Slack Integration](/integrations/slack/) for the manifest and OAuth flow.

## Create the app

```bash
curl -X POST http://localhost:9300/api/v1/apps \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Support Bot",
    "harness_id": "harness_...",
    "agent_id": "agent_...",
    "channel_type": "slack",
    "channel_config": {
      "signing_secret": "your-slack-signing-secret",
      "bot_token": "xoxb-your-bot-token",
      "session_strategy": "per_thread"
    }
  }'
```

The app starts in `draft` state. It does not accept incoming messages until you publish it.

## Choose a session strategy

`session_strategy` controls how incoming Slack messages map to Everruns sessions:

| Strategy | Behaviour | Use when |
|---|---|---|
| `per_thread` (default) | Each Slack thread is its own session | Support bots, Q&A, each thread is a separate conversation |
| `per_channel` | One session per channel | Persistent channel assistant, context shared across the channel |
| `per_user` | One session per user | Personal assistant, each user has their own ongoing chat |

## Publish

```bash
curl -X POST http://localhost:9300/api/v1/apps/$APP_ID/publish
```

After publishing, Slack webhooks are accepted at the app's endpoint. Configure your Slack app manifest with `https://your-host/api/v1/apps/{app_id}/slack/events` (verify the current path in the [API reference](/api/) under the Slack events endpoint before updating Slack, if the platform's URL shape changes, the API reference is authoritative).

## Unpublish and rollback

To stop accepting new messages without deleting the app:

```bash
curl -X POST http://localhost:9300/api/v1/apps/$APP_ID/unpublish
```

Existing sessions remain accessible via the API. The app moves back to `draft` and you can edit it before republishing.

## Pin an agent version

If you've enabled [Agent Versions](/features/agent-versions/), pick which version the app uses:

- `default`, follow the agent's default version.
- `latest`, always use the newest saved version.
- `pinned`, use a specific version until you change it.

Set this on the app's `agent_version_mode` field.

## See also

- [Apps feature](/features/apps/)
- [Slack Integration](/integrations/slack/), the prerequisite Slack-side configuration.
