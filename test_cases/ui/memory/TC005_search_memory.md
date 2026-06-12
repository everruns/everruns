# TC005: Search Memory by Name

## Description

Verify that the search input on the Memory page filters the list by name and surfaces the empty state when no memory match.

## Preconditions

- Server running (`just start-all`)
- User logged in
- At least three active memory exist with distinguishable names, including:
  - `tc005-alpha`
  - `tc005-beta`
  - `tc005-gamma`

## Test Data

| Field | Value |
|-------|-------|
| Search query (match) | `alpha` |
| Search query (no match) | `zzz-no-match` |

## Steps

1. Navigate to `/memory`
2. Type `alpha` into the **Search memory** input
3. Observe the memory grid
4. Clear the search field
5. Type `zzz-no-match` into the search input
6. Observe the page

## Expected Result

| Check | Expected |
|-------|----------|
| Filtered match | Only `tc005-alpha` is visible; `tc005-beta` and `tc005-gamma` are hidden |
| Clear restores | Clearing the search restores all three cards |
| No-match empty state | The "No memory found" empty state renders without a `New Memory` button (it shows only on the unfiltered empty state) |

## Cleanup

- Archive the `tc005-*` memory after the test.
