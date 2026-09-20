---
title: Publish an Agent to Slack
description: Add a Slack endpoint to an Agent, publish it, connect a Slack workspace, and verify the first message.
---

This guide deploys an Agent as a Slack bot through an Agent-owned endpoint. For Slack scopes, manual setup, and troubleshooting, see [Slack Integration](/integrations/slack/).

## Prerequisites

- An active Agent.
- A public HTTPS Everruns origin configured through `PUBLIC_APP_URL`.
- Permission to install an app in a Slack workspace.

## Add the Endpoint

1. Open the Agent and select **Integrations**.
2. Select **Add endpoint**, then select **Slack**.
3. Choose a session strategy and reply mode.
4. Leave the Slack credentials empty and select **Save endpoint**.

## Choose a session strategy

`session_strategy` controls how incoming Slack messages map to Everruns sessions:

| Strategy | Behaviour | Use when |
|---|---|---|
| `per_thread` (default) | Each Slack thread is its own session | Support bots, Q&A, each thread is a separate conversation |
| `per_channel` | One session per channel | Persistent channel assistant, context shared across the channel |
| `per_user` | One session per user | Personal assistant, each user has their own ongoing chat |

## Publish and Connect

1. Select **Publish** in the endpoint editor.
2. Select **Connect to Slack**.
3. Approve Slack's consent screen and choose a workspace.
4. If one-click setup is unavailable, return to **Integrations**, expand the endpoint, and select **Create Slack app**. Copy the resulting signing secret and bot token back through **Configure**.

Publish first because Slack verifies the manifest's endpoint Request URL when it creates the Slack app. New installs use `/v1/e/{endpoint_id}/slack/events`.

## Verify

1. In Slack, enter `/invite @botname` in a channel.
2. Mention the bot.
3. Return to the Agent's **Integrations** tab and expand the Slack endpoint.
4. Confirm that the checklist records the first message.

To stop new Slack messages without deleting the configuration, select **Unpublish** on this endpoint. Existing sessions remain available.

## See also

- [Slack Integration](/integrations/slack/), including scopes, manual setup, and troubleshooting.
- [Agent Versions](/features/agent-versions/), including endpoint version selection.
