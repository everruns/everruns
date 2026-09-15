---
type: Test Case
title: "TC001: Slack app setup — manifest, publish, webhook verification, first message"
description: "Verifies the Slack setup checklist can be completed end to end, that the generated manifest carries event_subscriptions so no hand-editing is needed, and that the checklist advances from observed webhook state."
tags:
  - everruns
  - test-case
  - ui
  - slack
---
# TC001: Slack app setup — manifest, publish, webhook verification, first message

## Description

Verifies the Slack setup checklist can be completed end to end against a real
Slack workspace: the generated manifest carries `event_subscriptions` so the
tester never opens Slack's Event Subscriptions page by hand, and the checklist
steps tick themselves from observed webhook state rather than from a local
guess.

This case exists because none of it is reachable from the integration suite.
`crates/server/tests/slack_integration_test.rs` asserts the manifest endpoint
returns YAML; it cannot assert that Slack *accepts* that YAML, which is the
only assertion that matters here.

## Preconditions

- A published Everruns deployment Slack can reach. Slack verifies the request
  URL when the manifest is saved, so `localhost` will not do: use a deployed
  environment or a tunnel.
- The tester can create Slack apps in a test workspace.
- An agent and harness exist to bind the app to.
- No existing Slack app for this Everruns app (this is a first-run case).

## Test Data

| Field | Value |
| --- | --- |
| App detail route | `/apps/{appId}` |
| Webhook path | `/v1/apps/{appId}/slack/events` |
| Manifest endpoint | `/v1/apps/{appId}/slack/manifest` |
| Expected bot events | `message.channels`, `message.groups`, `message.im`, `app_mention` |
| Test channel | any channel the tester can invite a bot to |

## Steps

1. Create an App with an agent and harness, and add a Slack channel.
2. **Before publishing**, attempt to create the Slack app. Note what the UI
   offers — publish must come first, because Slack verifies the request URL at
   manifest-save time.
3. Publish the app.
4. Click **Create Slack App**. Slack opens its "create from manifest" flow with
   the manifest pre-filled.
5. Read the manifest Slack is showing. Confirm it contains
   `settings.event_subscriptions.request_url` pointing at this app's webhook,
   and the four bot events from Test Data.
6. Create the app in Slack and install it to the workspace. Slack sends a
   `url_verification` challenge to the request URL as part of saving.
7. Return to the Everruns app detail page and reload.
8. Copy the Signing Secret (Basic Information → App Credentials) and Bot Token
   (OAuth & Permissions, `xoxb-…`) into the channel configuration and save.
9. In Slack, `/invite` the bot to the test channel and send it a message.
10. Reload the Everruns app detail page.

## Expected Result

- Step 2: the flow does not offer a manifest for an unpublished app, or states
  that publishing comes first. It must not hand out a manifest whose
  verification is guaranteed to fail.
- Step 5: `event_subscriptions` is present with the correct absolute URL and
  all four bot events. **The tester never opens Slack's Event Subscriptions
  page.** If this step requires hand-editing, the case fails.
- Step 6: Slack accepts the manifest without a request-URL error.
- Step 7: the "Configure Event Subscriptions" step shows as complete, driven by
  the recorded `webhook_verified_at`, not by anything the tester typed.
- Step 9: the agent replies in thread.
- Step 10: the final checklist step shows as complete, driven by
  `first_message_received_at`.

## Notes

The `/apps/{appId}` route is being retired — see
[Agent Exposure](../../../integrations/agent-exposure.md). When Slack
configuration moves to the agent's Integrations tab, the routes in Test Data
and the navigation in Steps 1–3 need updating. The Slack-side assertions
(Steps 5, 6, 9) are independent of where our UI puts the form and should
survive unchanged.
