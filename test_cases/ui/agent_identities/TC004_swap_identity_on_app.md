# TC004: Swap Identity on Existing App

## Description

Verify that an app's agent identity can be changed after creation.

## Preconditions

- UI running (dev or full mode)
- An app exists with an assigned identity
- A second agent identity exists

## Steps

1. Navigate to the app's edit page
2. Change the assigned identity to a different one
3. Save

## Expected Result

- App now shows the new identity
- Previous identity is no longer associated
