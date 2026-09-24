---
type: Test Case
title: "TC003: Slack endpoint setup"
description: "Verifies a Slack endpoint can start one-click install on save and receive a signed inbound event."
tags:
  - everruns
  - test-case
  - ui
  - agent-integrations
---
# TC003: Slack endpoint setup

## Description

Verifies a Slack endpoint can start one-click install on save and receive a signed inbound event.

## Preconditions

- A real Everruns stack is running with `AUTH_MODE=none`
- The Everruns API is reachable from Slack through a public HTTPS URL
- The deployment has a Slack app provisioner configured
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
2. Select **Slack** and verify **Configure manually** is collapsed.
3. Keep the credentials empty and click **Save endpoint**.
4. Verify Slack opens its consent screen without another Everruns action.
5. Approve the installation and verify the browser returns to the saved endpoint.
6. Verify **Connect to Slack** remains available for reconnecting.
7. Return to the Integrations tab and publish the Slack endpoint.
8. Invite the bot to a channel and send the test message.
9. Return to the expanded endpoint row.
10. On a deployment without a Slack app provisioner, repeat steps 1-2 and verify the manual fields start open.
11. Enter a manual signing secret before saving and verify Everruns saves the endpoint without starting Slack consent.

## Expected Result

- The endpoint is created without placeholder Slack credentials.
- Saving a credential-free endpoint starts Slack consent without a second button.
- A deployment without a provisioner keeps the manual setup flow open.
- Entering a manual credential does not start or overwrite the one-click flow.
- A failed install leaves the saved endpoint reachable with the failure reason and reconnect action visible.
- The inbound test message reaches the agent and the checklist records the first message.
