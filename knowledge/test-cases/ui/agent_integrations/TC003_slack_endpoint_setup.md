---
type: Test Case
title: "TC003: Slack endpoint setup"
description: "Verifies a Slack endpoint can create its app from an endpoint-scoped manifest and receive a signed inbound event."
tags:
  - everruns
  - test-case
  - ui
  - agent-integrations
---
# TC003: Slack endpoint setup

## Description

Verifies a Slack endpoint can create its app from an endpoint-scoped manifest and receive a signed inbound event.

## Preconditions

- A real Everruns stack is running with `AUTH_MODE=none`
- The Everruns API is reachable from Slack through a public HTTPS URL
- A Slack workspace is available for installing a test app
- An active agent exists

## Test Data

| Field | Value |
| --- | --- |
| Endpoint type | Slack |
| Session strategy | Per thread |
| Test message | `@Everruns setup verification` |

## Steps

1. Open the agent's **Integrations** tab and click **Add endpoint**.
2. Select **Slack**, keep the credentials empty, and create the endpoint.
3. Return to the Integrations tab and publish the Slack endpoint.
4. Expand the endpoint row.
5. Verify **Set up** appears before **Use it** and the setup checklist includes **Create a Slack app**.
6. Click **Create a Slack app**.
7. Verify Slack opens the create-from-manifest flow and the manifest's Event Subscriptions request URL contains `/v1/e/{endpointId}/slack/events`.
8. Create and install the Slack app.
9. Copy the Slack signing secret and bot token into the endpoint's **Configure** form, then save.
10. Invite the bot to a channel and send the test message.
11. Return to the expanded endpoint row.

## Expected Result

- The endpoint is created without placeholder Slack credentials.
- The setup checklist and endpoint-specific use guidance are both reachable from the expanded row.
- The **Create a Slack app** action uses the endpoint-scoped manifest route.
- Slack verifies the generated request URL without a hand-constructed API call.
- The inbound test message reaches the agent and the checklist records the first message.
