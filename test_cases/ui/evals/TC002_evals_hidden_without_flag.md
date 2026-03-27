# TC002: Evals - Hidden Without Feature Flag

## Description

Verify that the evals sidebar link is hidden when the `evals` feature flag is disabled.

## Preconditions

- Server running with `FEATURE_EVALS=false` (or production mode without explicit flag)
- User logged in

## Test Data

None.

## Steps

1. Observe the sidebar navigation
2. Look for "Evals" link in the Building Blocks section

## Expected Result

| Check | Expected |
|-------|----------|
| Sidebar | "Evals" link is NOT visible |
| Direct navigation | Navigating to `/evals` returns 404 or empty page (API returns no routes) |
