---
type: Test Case
title: "TC003: Slack endpoint setup"
description: "Verifies an organization connects Slack workspaces once, a Slack endpoint installs its own app into a chosen workspace on save, and receives a signed inbound event."
tags:
  - everruns
  - test-case
  - ui
  - agent-integrations
---
# TC003: Slack endpoint setup

## Description

Verifies an organization connects Slack workspaces once, a Slack endpoint installs its own app
into a chosen workspace on save, and receives a signed inbound event.

## Preconditions

- A real Everruns stack is running with `AUTH_MODE=none`
- The Everruns API is reachable from Slack through a public HTTPS URL
- `SECRETS_ENCRYPTION_KEY` is set, so the deployment provisions Slack apps
- Two Slack workspaces you can generate configuration tokens in, for the multi-workspace steps
- An active agent exists

## Test Data

| Field | Value |
| --- | --- |
| Endpoint type | Slack |
| Session strategy | Per thread |
| Test message | `@Everruns setup verification` |

## Steps

1. Open **Settings → Slack workspaces**. With nothing connected, verify the guided steps and the
   **Open api.slack.com/apps** link are shown.
2. Paste the configuration **access** token (`xoxe.xoxp-…`) and verify the form names the mistake
   without submitting it.
3. Paste the **refresh** token (`xoxe-1-…`) and verify the workspace connects without a button
   click and is listed by name and team ID.
4. Open the agent's **Integrations** tab, click **Add endpoint**, select **Slack**, and verify the
   connected workspace is shown with nothing to choose and **Configure manually** is collapsed.
5. Keep the credentials empty and click **Save endpoint**.
6. Verify Slack opens its consent screen with that workspace already selected.
7. Approve the installation and verify the endpoint shows its Slack connection status with an **Open in
   Slack** link that opens the agent's app.
8. Publish the endpoint, invite the bot to a channel, and send the test message.
9. Connect the second workspace in Settings, add another Slack endpoint, and verify the form asks
   which workspace and **Add to Slack** stays disabled until one is chosen.
10. Disconnect the first workspace and verify the confirmation says running agents keep working;
    then verify the first agent still answers in Slack.
11. On a deployment without `SECRETS_ENCRYPTION_KEY`, add a Slack endpoint and verify the manual
    fields start collapsed and Settings says the deployment does not create Slack apps.
12. Enter a manual signing secret before saving and verify Everruns saves the endpoint without
    starting Slack consent.

13. Return to the agent’s **Integrations** tab and verify the collapsed Slack card has a chevron
    and **Show configuration**. Expand it and verify connection status, conversation behavior,
    and **Configure manually** appear together, with no separate **Set up** checklist.
14. Change session strategy, collapse and reopen the card, and verify the unsaved selection remains.
    Click **Save changes** and verify the setting survives reload; credentials, app identity, and
    previously observed message delivery remain intact.
15. Enable the Slack agent pane and verify the form explains the required manifest update and
    reinstall. Save first, then open the updated manifest under **Configure manually**.
16. As a user without agent management permission, expand the card and verify connection details
    are visible without editable fields or save actions.

## Expected Result

- A workspace is connected once, from organization settings, and identified by name.
- An organization can connect more than one workspace; with one there is nothing to choose.
- Saving a credential-free endpoint starts Slack consent on the chosen workspace without a second
  button, and the agent is visibly live afterwards with a link into Slack.
- Disconnecting a workspace leaves agents already installed there working.
- A deployment that does not provision Slack apps keeps the manual setup flow available under **Configure manually**.
- Entering a manual credential does not start or overwrite the one-click flow.
- A failed install leaves the saved endpoint reachable with the failure reason and **Add to
  Slack** visible.
- The inbound test message reaches the agent and the connection status records **Message received**, even when no separate URL challenge was recorded.
