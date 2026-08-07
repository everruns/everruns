# TC002 Terminal Provider Errors

## Description

Verifies that utility-provider failures remain actionable in the agent Edit rail and that a
terminal health-check run replaces its live loading state without a page reload.

## Preconditions

- Development stack is running.
- At least one editable agent exists.
- `UTILITY_OPENAI_API_KEY` is configured for an account that returns
  `credit_balance_exhausted`.

## Steps

1. Open an editable agent and select Edit.
2. Confirm at least one built-in finding is visible in Checks, then select Analyze.
3. Wait for Analyze to finish.
4. Select Run health check and remain on the Edit page.
5. Wait for the health-check run to reach a terminal state.

## Expected Result

- Analyze reports that the AI provider account is out of credits or quota and suggests adding
  credits or raising provider limits.
- The Analyze error contains no provider response body, API key, token, or other secret detail.
- The built-in finding remains visible after Analyze fails.
- Health check changes live from Running / Generating and running cases to the same safe,
  actionable quota message without a page reload.
- The Run health check button is enabled again after the terminal failure.
