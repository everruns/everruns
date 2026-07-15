# TC002 New channel full-page route

## Description

Verifies Add channel opens a reload-safe full-page route with channel type cards, kind-specific configuration, and sticky footer actions.

## Preconditions

- A local (`AUTH_MODE=none`) or authenticated deployed Everruns UI is available
- The tester is signed in and has access to the target organization when authentication is enabled
- A draft or published App exists

## Test Data

| Field | Value |
| --- | --- |
| Route | `/apps/{appId}/channels/new` |
| Channel type | schedule |
| Cron preset | Hourly :30 |
| Timezone | `America/Chicago` |
| Message | `Run {{app.name}} now.` |

## Steps

1. Navigate to the App detail page.
2. Click Add channel.
3. Verify the browser URL is `/apps/{appId}/channels/new`.
4. Verify the five channel type cards are visible: Schedule, Webhook, AG-UI, FCP, Slack.
5. Select Schedule.
6. Choose the Hourly :30 preset.
7. Enter `America/Chicago` as the timezone.
8. Enter the invocation message.
9. Verify the summary sidebar displays a human-readable schedule preview and timezone.
10. Click Save channel.

## Expected Result

- The Add channel flow is not a dialog.
- Reloading the route preserves a valid page state.
- The schedule preview never shows the raw cron expression.
- Saving creates a channel and navigates to the full-page Edit Channel route for the new channel.
