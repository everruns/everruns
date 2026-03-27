# TC005: Evals - Delete Test Case

## Description

Verify that a test case can be deleted from an eval.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- Feature flag `evals` enabled (`FEATURE_EVALS=true`)
- An eval with at least one test case exists

## Test Data

None.

## Steps

1. Navigate to the eval detail page (`/evals/{eval_id}`)
2. Ensure the "Cases" tab is active
3. Note the current case count
4. Click the trash icon on one of the case cards
5. Wait for the case to disappear

## Expected Result

| Check | Expected |
|-------|----------|
| Trash icon | Visible on each case card |
| After delete | Case card removed from the grid |
| Case count | Header case count decrements by 1 |
| Empty state | If last case deleted, shows "No test cases yet" |
