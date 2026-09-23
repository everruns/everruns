---
title: Slack
description: Deploy Everruns agents as Slack bots that respond to messages, threads, and mentions. Configure Agent channel publishing, Slack installation, and conversation routing.
---

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="52.0" height="52.0" aria-hidden="true" style="float: right; margin-left: 16px;"><path d="M9 3.5L7 20.5M17 3.5l-2 17M4 8.5h16M3.2 15.5h16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>

Everruns connects an Agent to Slack through an Agent-owned **channel**. The channel receives Slack Events API requests, routes each conversation to a session, and posts the Agent's responses back to Slack.

In Everruns, a channel is a way an Agent is reachable, such as Slack, AG-UI, or A2A. This page calls it a *Slack Agent channel* where it could be confused with a Slack channel such as `#support`.

## What You Get

- **Conversational agents in Slack**: Users interact with the Agent in channels, threads, direct messages, or Slack's agent pane.
- **Session routing**: Conversations map to sessions by thread, channel, or user.
- **Secure webhooks**: Everruns verifies requests with Slack's signing secret.
- **Async responses**: Everruns acknowledges Slack immediately and posts the Agent's response when it is ready.
- **Per-channel Slack bots**: Each Slack Agent channel has its own Slack app, credentials, identity, and lifecycle.

## Before You Start

- Create an active Agent.
- Give Everruns a public HTTPS origin. Set `PUBLIC_APP_URL` to that origin and restart Everruns.
- Ask a Slack workspace administrator for permission to install an app.

Slack cannot verify `localhost`. For local development, expose Everruns through a public HTTPS tunnel before you create the Slack app.

## Connect an Agent to Slack

### 1. Create a Slack Agent Channel

1. Open the Agent.
2. Select **Integrations**.
3. Select **Add channel**.
4. Select **Slack**.
5. Choose the session strategy and reply mode. Leave the Slack credentials empty.
6. Select **Save channel**.

Everruns opens the channel editor after it saves the channel.

### 2. Publish the Channel

Select **Publish** in the channel editor. Publishing makes only this channel live.

Publish before you create the Slack app. The generated Slack manifest contains the channel's Request URL, and Slack verifies that URL when it creates the app.

### 3. Connect to Slack

Select **Connect to Slack** in the channel editor. Approve the Slack consent screen and choose a workspace. Everruns creates and installs the Slack app, then stores its signing secret, bot token, and workspace ID on this channel.

Some self-hosted deployments do not configure one-click Slack app creation. If Everruns reports that one-click setup is unavailable:

1. Return to the Agent's **Integrations** tab.
2. Expand the live Slack Agent channel.
3. Select **Create Slack app**.
4. Review Slack's pre-filled manifest and select **Create**.
5. Install the app to your workspace.
6. Copy the **Signing Secret** from **Basic Information**.
7. Copy the **Bot User OAuth Token** (`xoxb-...`) from **OAuth & Permissions**.
8. Select **Configure** on the channel, enter both values, and select **Save**.

The manifest already contains the bot scopes, event subscriptions, interactivity URL, and canonical channel-scoped Request URL:

```text
https://your-everruns-host/api/v1/e/{channel_id}/slack/events
```

Do not replace `{channel_id}` with an Agent ID or an App ID.

### 4. Invite and Test

1. In Slack, enter `/invite @botname` in a channel.
2. Mention the bot or send it a direct message.
3. Return to the Agent's **Integrations** tab and expand the Slack Agent channel.
4. Confirm that the setup checklist records the first message.

## Configure a Slack App Manually

Use this flow only when you cannot use **Connect to Slack** or **Create Slack app**.

1. Create and publish a Slack Agent channel from the Agent's **Integrations** tab.
2. Copy its Request URL from the expanded channel row.
3. In [Slack API Apps](https://api.slack.com/apps), select **Create New App** > **From scratch**.
4. Add these bot token scopes under **OAuth & Permissions**:
   - `chat:write`
   - `channels:history`
   - `groups:history`
   - `im:history`
   - `mpim:history`
   - `app_mentions:read`
   - `users:read`
5. Add these bot events under **Event Subscriptions**:
   - `message.channels`
   - `message.groups`
   - `message.im`
   - `message.mpim`
   - `app_mention`
6. Paste the channel Request URL into Slack's **Request URL** field.
7. Install the Slack app to your workspace.
8. Copy the signing secret and bot token into the channel's **Configure manually** fields.
9. Save the channel.

## Existing Installs

Existing Slack installs that use `/v1/apps/{app_id}/…` URLs continue to work. Everruns keeps those routes as permanent compatibility aliases. New installs use `/v1/e/{channel_id}/…`, which is the canonical channel-owned form.

## Channel Configuration

| Field | Required | Description |
|-------|----------|-------------|
| `signing_secret` | Before use | Slack app signing secret for HMAC-SHA256 verification. It can be empty while you create the channel, but the channel rejects all Slack requests until it is set. |
| `bot_token` | Before use | Bot User OAuth Token (`xoxb-...`) for sending responses. It can be empty while you create the channel. |
| `channel_id` | No | Restrict the Agent channel to one Slack channel, such as `C0123456789`. |
| `team_id` | No | Slack workspace ID. |
| `session_strategy` | No | `per_thread` by default, `per_channel`, or `per_user`. |
| `agent_surface_enabled` | No | `false` by default. Also serve Slack's agent pane; see [Agent Surface](#agent-surface). |

## Agent Surface

Slack apps can also appear as an **agent**, a dedicated assistant pane separate from channel conversations. Enabling `agent_surface_enabled` adds this pane without changing channel replies.

The generated manifest adds:

- the `features.agent_view` block with an `agent_description` derived from the Agent;
- the `assistant:write` bot scope; and
- the `app_home_opened`, `app_context_changed`, `agent_session_stopped`, and `agent_session_title_changed` bot events.

Enabling the pane on an existing Slack app requires a reinstall because a configuration change cannot grant the new `assistant:write` OAuth scope. Generate a fresh manifest, update the Slack app, and reinstall it. Channel replies continue while you do this.

Session strategy in the pane is always `per_thread`. The configured strategy still applies to channel conversations.

### Streaming Replies

Replies in the agent pane render as the Agent produces them. Channel threads receive one finished message instead of token-by-token updates.

## Session Strategies

| Strategy | Behavior | Tag Pattern |
|----------|----------|-------------|
| `per_thread` | Each Slack thread is a separate session. | `slack:thread:{thread_ts}` |
| `per_channel` | One session serves the channel. | `slack:channel:{channel}` |
| `per_user` | One session serves each Slack user. | `slack:user:{user}` |

Use `per_thread` for most Slack bots.

## How It Works

### Architecture

![Slack Architecture](../images/integrations/slack-architecture.svg)

### Message Flow

![Slack Message Flow](../images/integrations/slack-message-flow.svg)

**Inbound path:**

1. Slack posts a message event to `/v1/e/{channel_id}/slack/events`.
2. The channel verifies the signing secret and rejects duplicates.
3. Everruns acknowledges Slack within three seconds.
4. The channel finds or creates a session from the configured strategy.
5. Everruns creates a user message and starts an Agent turn.

**Outbound path:**

6. The Slack delivery dispatcher watches the turn.
7. The RuntimeAgent emits completed output messages.
8. The dispatcher posts each response with `chat.postMessage`.
9. Transient delivery failures retry with exponential backoff.
10. The dispatcher unregisters when the turn completes or fails.

## Troubleshooting

### URL Verification Failed

- Confirm that the channel is published.
- Confirm that `PUBLIC_APP_URL` is a public HTTPS origin and that Everruns restarted after the value changed.
- Confirm that the Request URL contains `/v1/e/{channel_id}/slack/events`.

### Bot Does Not Respond

- Confirm that the channel is published and enabled.
- Invite the bot to the channel with `/invite @botname`.
- Confirm that the Slack app has the required bot events and the `chat:write` scope.
- Confirm that the channel has the correct signing secret and bot token.

### Request Verification Failed

- Confirm that the channel's signing secret matches the value under the Slack app's **Basic Information** page.
- Confirm that the Everruns server clock is accurate.

## Approvals

When an Agent pauses for approval, Slack renders **Approve** and **Decline** buttons in the thread. Only the person whose message the Agent is answering can use them.

The generated manifest already points Slack interactivity at this Agent channel. A Slack app created before interactive approvals existed must save a fresh manifest once.

## Task Progress

When an Agent delegates work, Slack shows one **Tasks** message and updates it as workers finish. The message lists up to five tasks and summarizes larger groups by count.

## Links

- [Slack API Documentation](https://api.slack.com/docs)
- [Slack Events API](https://api.slack.com/events-api)
- [Publish an Agent to Slack](/how-to/publish-to-slack/)
- [Retired Apps compatibility](/features/apps/)
