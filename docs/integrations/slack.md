---
title: Slack
description: Deploy Everruns agents as Slack bots that respond to messages, threads, and mentions. Configure OAuth, event subscriptions, and channel routing for your workspace.
---

Everruns integrates with [Slack](https://slack.com) to deploy agents as bots that respond to messages in channels and threads. Messages are received via Slack's Events API, processed by the agent, and responses are posted back to the conversation.

## What You Get

- **Conversational agents in Slack**: Users interact with the agent by messaging in channels or threads
- **Session routing**: Conversations are mapped to sessions by thread, channel, or user
- **Secure webhooks**: Requests are verified using Slack's signing secret (HMAC-SHA256)
- **Async responses**: Slack is acknowledged immediately; the agent response is posted when ready
- **Per-app Slack bots**: Each Everruns App gets its own Slack App with its own identity, name, and avatar

## Quick Start (Manifest)

Everruns generates a pre-filled Slack App manifest for each app, making setup faster.

### 1. Create an App in Everruns

1. Go to **Apps** and click **New App**
2. Enter a name, select a Harness and Agent
3. Click **Create App** — you'll be redirected to the detail page

### 2. Create the Slack App

1. On the App detail page, click **Create Slack App**
2. This opens Slack's "Create app from manifest" page with pre-filled scopes and bot settings
3. Review the manifest and click **Create**
4. Install the app to your workspace when prompted

### 3. Copy Credentials Back

1. In your new Slack app, go to **Basic Information** and copy the **Signing Secret**
2. Go to **OAuth & Permissions** and copy the **Bot User OAuth Token** (`xoxb-...`)
3. Back in Everruns, click **Configure** on the Slack Integration card
4. Paste both values and click **Save**

### 4. Configure Event Subscriptions

1. **Publish** the app in Everruns first (so the webhook URL is live)
2. Copy the **Request URL** shown on the app detail page
3. In your Slack app settings, go to **Event Subscriptions** → Enable Events
4. Paste the Request URL — Slack will verify it automatically
5. Subscribe to bot events: `message.channels`, `message.groups`, `message.im`, `message.mpim`, `app_mention`
6. Click **Save Changes**

> **Note:** Event subscriptions require a live webhook URL, so the Everruns app must be published before configuring this step.

### 5. Start Using

Invite the bot to a channel (`/invite @botname`) and send a message. The bot will respond using the configured agent.

## Quick Start (Manual)

If you prefer to set up everything manually without the manifest:

### 1. Create a Slack App

1. Go to [api.slack.com/apps](https://api.slack.com/apps) and click **Create New App** > **From scratch**
2. Name your app and select the workspace
3. Navigate to **OAuth & Permissions** and add these **Bot Token Scopes**:
   - `chat:write` — Send messages
   - `channels:history` — Read messages in public channels
   - `groups:history` — Read messages in private channels (optional)
   - `im:history` — Read direct messages (optional)
   - `mpim:history` — Read group direct messages (optional)
   - `app_mentions:read` — React to @mentions (optional)
   - `users:read` — Resolve user display names
4. Click **Install to Workspace** and authorize
5. Copy the **Bot User OAuth Token** (`xoxb-...`) from the OAuth page

### 2. Get the Signing Secret

1. In your Slack app settings, go to **Basic Information**
2. Under **App Credentials**, copy the **Signing Secret**

### 3. Create an App in Everruns

You need a Harness and Agent already configured. Then create an App via the UI or API:

**Via UI:**

1. Go to **Apps** and click **New App**
2. Enter a name (e.g., "Support Bot")
3. Select your Harness and Agent
4. Click **Create App**
5. On the detail page, click **Configure** under Slack Integration
6. Paste the **Signing Secret** and **Bot Token**
7. Choose a session strategy (default: `per_thread`)
8. Click **Save**

**Via API:**

```bash
curl -X POST http://localhost:9300/api/v1/apps \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Support Bot",
    "harness_id": "harness_...",
    "agent_id": "agent_...",
    "channel_type": "slack",
    "channel_config": {
      "signing_secret": "your-signing-secret",
      "bot_token": "xoxb-your-bot-token",
      "session_strategy": "per_thread"
    }
  }'
```

### 4. Publish the App

Publish to start accepting messages:

```bash
curl -X POST http://localhost:9300/api/v1/apps/{app_id}/publish
```

Or click **Publish** in the UI.

### 5. Configure the Slack Webhook

1. Copy the webhook URL from the App detail page. It follows this format:
   ```
   https://your-everruns-host/api/v1/apps/{app_id}/slack/events
   ```
2. In your Slack app settings, go to **Event Subscriptions**
3. Turn on **Enable Events**
4. Paste the webhook URL as the **Request URL** — Slack will send a verification challenge that Everruns handles automatically
5. Under **Subscribe to bot events**, add:
   - `message.channels` — Messages in public channels
   - `message.groups` — Messages in private channels (optional)
   - `message.im` — Direct messages (optional)
   - `app_mention` — @mentions (optional)
6. Click **Save Changes**

:::note[Public URL Required]
The webhook URL must be publicly accessible. If running locally, use a tool like [ngrok](https://ngrok.com) to expose your local Everruns instance.
:::

## Channel Config Reference

| Field | Required | Description |
|-------|----------|-------------|
| `signing_secret` | Yes | Slack app signing secret for HMAC-SHA256 verification |
| `bot_token` | Yes | Bot User OAuth Token (`xoxb-...`) for sending responses |
| `channel_id` | No | Restrict to a specific channel (e.g., `C0123456789`) |
| `team_id` | No | Slack workspace ID |
| `session_strategy` | No | `per_thread` (default), `per_channel`, or `per_user` |

## Session Strategies

The session strategy controls how Slack messages map to Everruns sessions:

| Strategy | Behavior | Tag Pattern |
|----------|----------|-------------|
| `per_thread` | Each Slack thread is a separate session | `slack:thread:{thread_ts}` |
| `per_channel` | One session per Slack channel | `slack:channel:{channel}` |
| `per_user` | One session per Slack user | `slack:user:{user}` |

**`per_thread`** is recommended for most use cases — it gives each conversation its own context, matching how Slack threads naturally work.

## How It Works

### Architecture

![Slack Architecture](../images/integrations/slack-architecture.svg)

### Message Flow

![Slack Message Flow](../images/integrations/slack-message-flow.svg)

**Inbound path:**

1. Slack sends a message event to the per-app webhook endpoint
2. Everruns verifies the request using the HMAC-SHA256 signing secret
3. Duplicate events are skipped (Slack sends both `app_mention` and `message` for @mentions)
4. The request is acknowledged immediately — Slack requires a response within 3 seconds
5. A session is found or created based on session strategy tags (e.g., `slack:thread:{ts}`)
6. A user message is created, triggering an agent turn

**Outbound path:**

7. The `SlackDeliveryDispatcher` registers to watch for events from this turn
8. As the agent produces responses, `output.message.completed` events are broadcast via PostgreSQL NOTIFY
9. The dispatcher posts each response to Slack using `chat.postMessage` with the bot token
10. On transient failures, delivery is retried with exponential backoff (up to 3 attempts)
11. When the turn completes or fails, the dispatcher unregisters

## Troubleshooting

### "URL verification failed" when setting up Event Subscriptions
- Ensure the app is **published** in Everruns before configuring the webhook in Slack
- Check that the webhook URL is correct and publicly accessible

### Bot not responding to messages
- Verify the app is in **published** status
- Check that the bot has been invited to the channel (`/invite @botname`)
- Ensure the correct bot events are subscribed (`message.channels`, etc.)
- Verify the bot token has `chat:write` scope

### "Request verification failed" errors
- Confirm the signing secret in Everruns matches the one in Slack's **Basic Information** page
- Check that your server clock is accurate (signing verification uses timestamps)

## Overview Video

<iframe
  width="100%"
  style="aspect-ratio: 16 / 9; border-radius: 8px; margin-top: 1rem;"
  src="https://www.youtube.com/embed/RuNeh8i6Bdk"
  title="Slack Integration Overview"
  frameborder="0"
  allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture"
  allowfullscreen>
</iframe>

## Links

- [Slack API Documentation](https://api.slack.com/docs)
- [Slack Events API](https://api.slack.com/events-api)
- [Apps Feature Guide](/features/apps/)
