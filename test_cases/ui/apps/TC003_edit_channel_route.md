# TC003 Edit channel full-page route

## Description

Verifies the Edit Channel page supports webhook editing, pause/enable, delete, and sticky footer actions on a deep-linkable route.

## Preconditions

- A local (`AUTH_MODE=none`) or authenticated deployed Everruns UI is available
- The tester is signed in and has access to the target organization when authentication is enabled
- A published App exists with an enabled webhook channel

## Test Data

| Field | Value |
| --- | --- |
| Route | `/apps/{appId}/channels/{channelId}` |
| Updated session mode | New session per invocation |

## Steps

1. Navigate directly to `/apps/{appId}/channels/{channelId}`.
2. Verify the header shows channel kind and active status.
3. Verify the Invocation tab is available for the webhook channel.
4. Change the session mode.
5. Click Save.
6. Re-open the Edit Channel route.
7. Click Pause and verify the status changes to paused.
8. Click Enable and verify the status changes to active.
9. Click Delete and confirm the channel no longer appears on the App detail page.

## Expected Result

- The edit flow is not a dialog and is reload-safe.
- Pause/Enable updates channel `enabled` state.
- Delete returns to the App detail page with the channel removed.
