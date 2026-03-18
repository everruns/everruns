---
title: Apps
description: Deploy Everruns agents to external channels like Slack, WhatsApp, and custom webhooks. Configure distribution, authentication, and message routing per app.
---

An App binds a Harness and Agent to a distribution channel, turning your agent into a deployed service that responds to external messages. Apps provide a publish/unpublish lifecycle — only published apps accept incoming requests.

## How It Works

```mermaid
graph LR
    Channel["External Channel<br/>(Slack, etc.)"] -->|webhook| App
    App -->|references| Harness
    App -->|references| Agent
    App -->|creates/finds| Session
    Session -->|runs| RuntimeAgent

    classDef external fill:#ffd6a5,stroke:#e07b39,color:#5a3000
    classDef app fill:#e8daef,stroke:#7d3c98,color:#4a235a
    classDef config fill:#c7f0db,stroke:#2d6a4f,color:#1b4332
    classDef runtime fill:#bde0fe,stroke:#3a86a8,color:#023047

    class Channel external
    class App app
    class Harness,Agent config
    class Session,RuntimeAgent runtime
```

1. An external channel (e.g., Slack) sends a webhook to the App's endpoint
2. The App verifies the request using channel-specific security (e.g., Slack signing secret)
3. A session is found or created based on the configured session strategy
4. The agent processes the message and responds through the channel

## Channel Types

Each app has a single channel type with channel-specific configuration stored as JSON.

| Channel | Status | Description |
|---------|--------|-------------|
| Slack | Available | Deploy agents as Slack bots |
| WhatsApp | Planned | — |
| Web Widget | Planned | — |

See [Slack Integration](/integrations/slack/) for a step-by-step setup guide.

## Lifecycle

Apps follow a draft/published lifecycle:

- **Draft**: App is configured but does not accept incoming requests. Use this state while setting up or testing.
- **Published**: App is live and accepting incoming messages. Webhook requests are processed.
- **Archived**: App is soft-deleted and hidden from listings.

Unpublishing an app stops new message processing. Existing sessions remain accessible.

## Session Strategies

Session strategies control how incoming messages map to sessions. The right strategy depends on your use case:

| Strategy | Behavior | Best for |
|----------|----------|----------|
| `per_thread` (default) | Each conversation thread gets its own session | Support bots, Q&A |
| `per_channel` | One session per channel | Persistent channel assistants |
| `per_user` | One session per user | Personal assistants |

## Managing Apps

### Via UI

1. Navigate to **Apps** in the sidebar
2. Click **Create App**
3. Fill in name, select a harness and agent, configure the channel
4. Click **Create**
5. Use **Publish** to make the app live

### Via API

Create an app:

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

Publish an app:

```bash
curl -X POST http://localhost:9300/api/v1/apps/{app_id}/publish
```

List apps:

```bash
curl http://localhost:9300/api/v1/apps
```

## API Reference

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/apps` | Create app |
| GET | `/v1/apps` | List apps |
| GET | `/v1/apps/{app_id}` | Get app |
| PATCH | `/v1/apps/{app_id}` | Update app |
| DELETE | `/v1/apps/{app_id}` | Archive app |
| POST | `/v1/apps/{app_id}/publish` | Publish app |
| POST | `/v1/apps/{app_id}/unpublish` | Unpublish app |
