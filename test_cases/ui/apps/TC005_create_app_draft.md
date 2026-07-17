# TC005 Create App draft

## Description

Verify that an App draft can be created before channel configuration and opens in the channels-first detail view.

## Preconditions

- A local (`AUTH_MODE=none`) or authenticated deployed Everruns UI is available
- The tester is signed in and has access to the target organization when authentication is enabled
- An active agent and harness are available

## Test Data

| Field | Value |
| --- | --- |
| App name | Functional Draft App |
| Agent | Any active test agent |
| Harness | Generic |

## Steps

1. Navigate to `/apps`.
2. Click Create App.
3. Enter the App name and select the active agent.
4. Confirm the harness selection is populated for the selected agent, then create the App.
5. Reload the resulting App detail route.
6. Click Add channel.

## Expected Result

- A draft App is created and its detail route remains valid after reload.
- The detail header identifies the selected agent and harness.
- The App has no channels until one is explicitly added.
- Add channel opens `/apps/{appId}/channels/new` with Webhook, AG-UI, FCP, and Slack choices; Schedule is absent.
