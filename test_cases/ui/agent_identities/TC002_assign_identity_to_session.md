# TC002: Assign Identity to Session

## Description

Verify that an agent identity can be assigned to a session during creation.

## Preconditions

- UI running (dev or full mode)
- At least one agent identity exists
- At least one agent exists

## Steps

1. Navigate to create a new session
2. Select an agent
3. Assign the existing identity in the creation dialog
4. Submit

## Expected Result

- Session is created with the assigned identity
- Session detail shows the identity association
