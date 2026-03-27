# TC007: Evals - Runs Tab

## Description

Verify that the Runs tab on the eval detail page lists previous runs with status and metrics.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- Feature flag `evals` enabled (`FEATURE_EVALS=true`)
- An eval with at least one completed run exists

## Test Data

None.

## Steps

1. Navigate to the eval detail page (`/evals/{eval_id}`)
2. Click the "Runs" tab
3. Observe the runs list

## Expected Result

| Check | Expected |
|-------|----------|
| Runs tab | Shows count of runs in tab label |
| Run rows | Each run shows status badge, timestamp, triggered-by |
| Metrics | Completed runs show pass rate and passed/total count |
| Clickable | Clicking a run navigates to `/evals/{eval_id}/runs/{run_id}` |
| Ordering | Most recent run appears first |
