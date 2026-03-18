# TC003: Setup Page Shows Animated Progress

## Description

Verify that the org setup page displays three sequential setup steps with animated completion.

## Preconditions

- User just created a new organisation and was redirected to `/orgs/<orgId>/setup`

## Steps

1. Observe the setup page immediately after redirect
2. Wait for all three steps to animate to completion
3. Note the final state of the page

## Expected Result

- Page header shows "Setting up <org name>" with a Building icon
- Three steps appear in sequence:
  1. "Organisation created" — "Your new organisation has been provisioned"
  2. "Harnesses initialised" — "Built-in harnesses (Base, Generic, Platform Chat) are ready"
  3. "Default settings configured" — "Default and base harnesses have been assigned"
- Each step transitions from pending (faded) to a spinner to a green checkmark
- After all steps complete, a "Go to dashboard" button appears
