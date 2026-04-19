# TC003: Setup Page Shows Animated Progress

## Description

Verify that the org setup page displays four sequential setup steps with animated completion, then offers the supported onboarding provider choices.

## Preconditions

- User just created a new organisation and was redirected to `/orgs/<orgId>/setup`

## Steps

1. Observe the setup page immediately after redirect
2. Wait for all four steps to animate to completion
3. Review the provider choices shown below the completed steps
4. Note the final state of the page

## Expected Result

- Page header shows "Setting up <org name>" with a Building icon
- Four steps appear in sequence:
  1. "Organisation created" — "Your new organisation has been provisioned"
  2. "Harnesses initialised" — "Built-in harnesses are ready"
  3. "Default settings configured" — "Default and base harnesses have been assigned"
  4. "LLM provider configured" — "API provider and credentials set up"
- Each step transitions from pending (faded) to a spinner to a green checkmark
- After all steps complete, the page shows "Select your LLM provider"
- Provider choices are limited to "OpenAI" and "Anthropic"
- "Azure OpenAI" is not shown on the org setup page
- The page still offers "Skip for now" and "Continue"
