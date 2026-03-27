# TC006: Evals - Start Eval Run

## Description

Verify that an eval run can be triggered and the run detail page displays correctly.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- Feature flag `evals` enabled (`FEATURE_EVALS=true`)
- An eval with at least one test case exists
- An LLM provider is configured

## Test Data

None.

## Steps

1. Navigate to the eval detail page (`/evals/{eval_id}`)
2. Click "Run Eval" button
3. Wait for redirect to the run detail page (`/evals/{eval_id}/runs/{run_id}`)
4. Observe the run status and summary cards

## Expected Result

| Check | Expected |
|-------|----------|
| Run button | "Run Eval" button visible and enabled for active evals |
| Redirect | After clicking, redirects to run detail page |
| Run status | Shows "pending" or "running" initially |
| Summary cards | Displays pass rate, results count, latency, tokens placeholders |
| Results table | Shows individual case results as they complete |
