---
type: Test Case
title: "TC003: Slack channel setup"
description: "Verifies a Slack channel can create its app from a channel-scoped manifest and receive a signed inbound event."
tags:
  - everruns
  - test-case
  - ui
  - agent-integrations
---
# TC003: Slack channel setup

## Description

Verifies a Slack channel can create its app from a channel-scoped manifest and receive a signed inbound event.

## Preconditions

- A real Everruns stack is running with `AUTH_MODE=none`
- The Everruns API is reachable from Slack through a public HTTPS URL
- A Slack workspace is available for installing a test app
- An active agent exists

## Test Data

| Field | Value |
| --- | --- |
| Channel type | Slack |
| Session strategy | Per thread |
| Test message | `@Everruns setup verification` |

## Steps

1. Open the agent's **Integrations** tab and click **Add channel**.
2. Select **Slack**, keep the credentials empty, and create the channel.
3. Return to the Integrations tab and publish the Slack channel.
4. Expand the channel row.
5. Verify **Set up** appears before **Use it** and the setup checklist includes **Create a Slack app**.
6. Click **Create a Slack app**.
7. Verify Slack opens the create-from-manifest flow and the manifest's Event Subscriptions request URL contains `/v1/e/{channelId}/slack/events`.
8. Create and install the Slack app.
9. Copy the Slack signing secret and bot token into the channel's **Configure** form, then save.
10. Invite the bot to a channel and send the test message.
11. Return to the expanded channel row.

## Expected Result

- The channel is created without placeholder Slack credentials.
- The setup checklist and channel-specific use guidance are both reachable from the expanded row.
- The **Create a Slack app** action uses the channel-scoped manifest route.
- Slack verifies the generated request URL without a hand-constructed API call.
- The inbound test message reaches the agent and the checklist records the first message.
