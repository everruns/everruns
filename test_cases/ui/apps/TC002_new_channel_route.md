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
| Channel type | webhook |

## Steps

1. Navigate to the App detail page.
2. Click Add channel.
3. Verify the browser URL is `/apps/{appId}/channels/new`.
4. Verify Webhook, AG-UI, FCP, and Slack channel type cards are visible.
5. Verify Schedule is not offered and the page directs scheduled automation to the agent's Triggers tab.
6. Select Webhook.
7. Click Save channel.

## Expected Result

- The Add channel flow is not a dialog.
- Reloading the route preserves a valid page state.
- Schedule is not available as a new App channel.
- Saving creates a channel and navigates to the full-page Edit Channel route for the new channel.
