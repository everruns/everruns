# TC004: Evals - Add Test Case

## Description

Verify that a test case can be added to an existing eval with messages and scorers.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- Feature flag `evals` enabled (`FEATURE_EVALS=true`)
- An eval already exists (from TC003 or created via API)

## Test Data

| Field | Value |
|-------|-------|
| Case Name | `Greeting test` |
| Description | `Agent should greet the user` |
| Messages | `Hello, who are you?` |
| Scorer type | `contains` |
| Scorer value | `hello` |

## Steps

1. Navigate to the eval detail page (`/evals/{eval_id}`)
2. Ensure the "Cases" tab is active
3. Click "Add Case" button
4. Fill in Name: `Greeting test`
5. Fill in Description: `Agent should greet the user`
6. Enter message: `Hello, who are you?`
7. Select scorer type: `contains`
8. Enter scorer value: `hello`
9. Click "Add Case" submit button
10. Wait for the case to appear in the cases grid

## Expected Result

| Check | Expected |
|-------|----------|
| Add Case form | Shows name, description, messages, scorer fields |
| After submit | Form clears and case appears in grid |
| Case card | Shows name, description, message content, scorer badge |
| Scorer badge | Displays `contains("hello")` |
| Case count | Header case count increments by 1 |
