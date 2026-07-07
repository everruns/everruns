# TC003: Assign Identity to App

## Description

Verify that an agent identity can be assigned to an app from the app detail
page. Identity is no longer set during app creation; it is configured after the
draft exists, alongside channels.

## Preconditions

- UI running (dev or full mode)
- At least one agent identity exists

## Steps

1. Create a new app (name + harness) and submit.
2. On the app detail page, locate the "Agent identity" control in the right rail.
3. Select the existing identity.

## Expected Result

- The identity selection persists (reloading the detail page shows it selected).
- App detail shows the identity association.
