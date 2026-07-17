# TC001 App detail channels-first view

## Description

Verifies the App detail page renders a channels-first operations view after App schedules have moved to agent triggers.

## Preconditions

- A local (`AUTH_MODE=none`) or authenticated deployed Everruns UI is available
- The tester is signed in and has access to the target organization when authentication is enabled
- A published App exists with a webhook channel

## Test Data

| Field | Value |
| --- | --- |
| App name | Dad Joke Service |
| Channel type | webhook |

## Steps

1. Navigate to `/apps`.
2. Open the App detail page.
3. Verify the header shows the App name, lifecycle pill, agent/harness references, and actions.
4. Verify the stat strip shows Health, Invocations 24h, Success rate, and Activity.
5. Verify the Channels section renders the webhook channel row.
6. Verify no Schedule channel is present.
7. Click the channel row to expand it.
8. Open the channel kebab and verify Configure links to `/apps/{appId}/channels/{channelId}`.

## Expected Result

- The App detail page uses the channels-first V2 layout.
- App schedules do not appear as channels after migration.
- Configure deep-links to the full-page Edit Channel route.
