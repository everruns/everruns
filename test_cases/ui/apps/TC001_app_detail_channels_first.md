# TC001 App detail channels-first view

## Description

Verifies the App detail page renders a channels-first operations view with no raw cron expressions outside editable cron inputs.

## Preconditions

- `AUTH_MODE=none`
- `just start-dev --no-watch` is running
- A published App exists with a schedule channel using cron `0 30 * * * * *` and timezone `America/Chicago`

## Test Data

| Field | Value |
| --- | --- |
| App name | Dad Joke Hourly |
| Channel type | schedule |
| Cron | `0 30 * * * * *` |
| Timezone | `America/Chicago` |

## Steps

1. Navigate to `/apps`.
2. Open the App detail page.
3. Verify the header shows the App name, lifecycle pill, agent/harness references, and actions.
4. Verify the stat strip shows Health, Invocations 24h, Success rate, and Activity.
5. Verify the Channels section renders the schedule channel row.
6. Verify the row subline uses a human-readable schedule label with timezone.
7. Verify the raw cron expression is not visible anywhere on the detail page.
8. Click the channel row to expand it.
9. Open the channel kebab and verify Configure links to `/apps/{appId}/channels/{channelId}`.

## Expected Result

- The App detail page uses the channels-first V2 layout.
- The schedule row displays a human-readable cadence such as `At 30 minutes past the hour · America/Chicago`.
- The raw cron expression is not visible outside the edit form.
- Configure deep-links to the full-page Edit Channel route.
