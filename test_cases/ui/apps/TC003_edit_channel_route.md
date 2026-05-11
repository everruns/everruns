# TC003 Edit channel full-page route

## Description

Verifies the Edit Channel page supports schedule editing, pause/enable, run now, delete, and sticky footer actions on a deep-linkable route.

## Preconditions

- `AUTH_MODE=none`
- `FEATURE_APPS_DETAIL_V2=true`
- `just start-dev --no-watch` is running
- A published App exists with an enabled schedule channel

## Test Data

| Field | Value |
| --- | --- |
| Route | `/apps/{appId}/channels/{channelId}` |
| Updated cron preset | Daily 09:00 |
| Updated timezone | `UTC` |

## Steps

1. Navigate directly to `/apps/{appId}/channels/{channelId}`.
2. Verify the header shows channel kind, active status, and a human-readable schedule subline.
3. Verify the Schedule tab is selected for a schedule channel.
4. Change the schedule preset and timezone.
5. Click Save.
6. Re-open the Edit Channel route.
7. Click Pause and verify the status changes to paused.
8. Click Enable and verify the status changes to active.
9. Click Run now.
10. Click Delete and confirm the channel no longer appears on the App detail page.

## Expected Result

- The edit flow is not a dialog and is reload-safe.
- The schedule tab uses `<CronInput>` and is the only place raw cron appears.
- Header and breadcrumbs use human-readable schedule labels.
- Pause/Enable updates channel `enabled` state.
- Run now invokes the schedule trigger route for published enabled schedule channels.
- Delete returns to the App detail page with the channel removed.
