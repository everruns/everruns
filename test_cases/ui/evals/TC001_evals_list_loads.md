# TC001: Evals - List Page Loads

## Description

Verify that the evals list page loads successfully and displays the eval cards grid with experimental badge.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- Feature flag `evals` enabled (`FEATURE_EVALS=true`)

## Test Data

None.

## Steps

1. Navigate to `/evals` via the sidebar
2. Wait for loading to complete
3. Observe the page header and content area

## Expected Result

| Check | Expected |
|-------|----------|
| Sidebar | "Evals" link visible with experimental flask icon |
| Page header | Shows "Evals" with experimental badge |
| New Eval button | "New Eval" button visible in header |
| No error | No error messages displayed |
| Empty state | If no evals exist, shows empty state with prompt to create |
