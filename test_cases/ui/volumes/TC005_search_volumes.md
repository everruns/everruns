# TC005: Search Volumes by Name

## Description

Verify that the search input on the Volumes page filters the list by name and surfaces the empty state when no volumes match.

## Preconditions

- Server running (`just start-all`)
- User logged in
- At least three active volumes exist with distinguishable names, including:
  - `tc005-alpha`
  - `tc005-beta`
  - `tc005-gamma`

## Test Data

| Field | Value |
|-------|-------|
| Search query (match) | `alpha` |
| Search query (no match) | `zzz-no-match` |

## Steps

1. Navigate to `/volumes`
2. Type `alpha` into the **Search volumes** input
3. Observe the volume grid
4. Clear the search field
5. Type `zzz-no-match` into the search input
6. Observe the page

## Expected Result

| Check | Expected |
|-------|----------|
| Filtered match | Only `tc005-alpha` is visible; `tc005-beta` and `tc005-gamma` are hidden |
| Clear restores | Clearing the search restores all three cards |
| No-match empty state | The "No volumes found" empty state renders without a `New Volume` button (it shows only on the unfiltered empty state) |

## Cleanup

- Archive the `tc005-*` volumes after the test.
