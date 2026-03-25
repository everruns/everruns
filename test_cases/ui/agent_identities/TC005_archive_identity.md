# TC005: Archive Identity

## Description

Verify that archiving an identity preserves historical references but prevents new assignments.

## Preconditions

- UI running (dev or full mode)
- An agent identity exists and is assigned to at least one session or app

## Steps

1. Navigate to the identity's detail page
2. Archive the identity
3. Verify the identity still appears in historical session/app records
4. Attempt to assign the archived identity to a new session or app

## Expected Result

- Identity is marked as archived
- Historical references (existing sessions/apps) still display the identity
- Archived identity is not available for selection when creating new sessions or apps
