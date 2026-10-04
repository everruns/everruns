---
title: Publish an Agent to Slack
description: Add a Slack channel to an Agent, publish it, connect a Slack workspace, and verify the first message.
appliesTo: [platform, cloud]
---

This guide deploys an Agent as a Slack bot through an Agent-owned channel. For Slack scopes, manual setup, and troubleshooting, see [Slack Integration](/capabilities/slack/).

## Prerequisites

- An active Agent.
- A public HTTPS Everruns origin configured through `PUBLIC_APP_URL`.
- Permission to install an app in a Slack workspace.

## Add the Channel

1. Open the Agent and select **Integrations**.
2. Select **Add channel**, then select **Slack**.
3. Choose a session strategy and reply mode.
4. Leave the Slack credentials empty and select **Save channel**.

## Choose a session strategy

`session_strategy` controls how incoming Slack messages map to Everruns sessions:

| Strategy | Behaviour | Use when |
|---|---|---|
| `per_thread` (default) | Each Slack thread is its own session | Support bots, Q&A, each thread is a separate conversation |
| `per_channel` | One session per channel | Persistent channel assistant, context shared across the channel |
| `per_user` | One session per user | Personal assistant, each user has their own ongoing chat |

## Choose when the agent responds

The Slack endpoint editor shows a **Response policy** selector:

- **All messages** preserves the existing behavior and is the default.
- **Mentions only** responds to direct messages and explicit `@mentions`.
- **Relevant messages** also responds to clear requests within the agent's purpose,
  including contextual thread follow-ups. Unrelated and uncertain messages stay silent.

Relevant messages requires a configured deployment Decisions service. For Jev, set
`UTILITY_TYPESAFE_API_KEY` and select `DECISIONS_DRIVER=typesafe`. Mentions and direct
messages work without a classifier. A missing classifier or a failed decision leaves
unmentioned messages silent, without posting an acknowledgement or running the agent.

Reply mode still controls what an accepted turn posts to Slack. Response policy
controls whether that turn starts. Select **All messages** to restore the previous
behavior.

## Publish and Connect

Your organization must have a Slack workspace connected in **Settings** > **Slack workspaces** first; an administrator does this once. See [Connect a Slack Workspace](/capabilities/slack/#connect-a-slack-workspace).

1. Select **Publish** in the channel editor.
2. If more than one workspace is connected, choose which one.
3. Select **Add to Slack**.
4. Approve Slack's consent screen. The workspace is already selected.
5. If one-click setup is unavailable, return to **Integrations**, expand the channel, and select **Create Slack app**. Copy the resulting signing secret and bot token back through **Configure**.

Publish first because Slack verifies the manifest's channel Request URL when it creates the Slack app. New installs use `/v1/channels/{channel_id}/slack/events`.

## Verify

1. In Slack, enter `/invite @botname` in a channel.
2. Mention the bot.
3. Return to the Agent's **Integrations** tab and expand the Slack channel.
4. Confirm that the checklist records the first message.

To stop new Slack messages without deleting the configuration, select **Unpublish** on this channel. Existing sessions remain available.

## Resolve installation issues

Open **Settings** > **Health** to review pending Slack permission or credential issues.
The same warning appears beside the endpoint and under **Action required** in notifications
when notifications are enabled. Reading or snoozing an announcement leaves the issue open.

For an app Everruns created, select **Reconnect Slack** and approve the additional permissions.
For a manually configured app, add the listed bot scopes in Slack, reinstall the existing app,
and save the refreshed bot token in the endpoint editor. Your Slack administrator may need
to approve the change. Use **Check again** to verify the installation; unavailable or stale
permission evidence keeps the issue open until a current check succeeds.

## See also

- [Slack Integration](/capabilities/slack/), including scopes, manual setup, and troubleshooting.
- [Agent Versions](/features/agent-versions/), including channel version selection.
