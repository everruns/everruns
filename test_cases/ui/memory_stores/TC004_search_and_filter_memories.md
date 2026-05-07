# TC004: Search and Filter Memories

## Description

Verify that the memory list supports text search across memory content and filtering by kind, and that the result count updates accordingly.

## Preconditions

- UI running and user authenticated
- A store exists that has at least 3 memories of different kinds. The easiest setup:
  - Run an agent with the `memory` capability that calls `remember` a few times, e.g. one `fact`, one `preference`, one `correction`
  - Or seed via API: `POST /v1/memory-stores/{store_id}/memories` is not exposed; use the `remember` tool from an agent session against the target store
- Memories should have distinguishable content (e.g. one mentioning "espresso", one mentioning "dark mode", one mentioning "PostgreSQL")

## Test Data

| Field | Value |
|-------|-------|
| Search query 1 | espresso |
| Search query 2 | nonexistent-substring-xyz |
| Kind filter 1 | All kinds |
| Kind filter 2 | Preference |

## Steps

1. Navigate to **Memory** and select the seeded store from the left list
2. Confirm the right pane shows the count "N memories" matching the seed count
3. Type `espresso` in the search input
4. Observe the filtered list and counter
5. Clear the search input
6. Open the **Kind** dropdown and pick **Preference**
7. Observe the filtered list and counter
8. Set **Kind** back to **All kinds**
9. Type `nonexistent-substring-xyz` in the search input

## Expected Result

- Step 2: All seeded memories are listed; the counter shows "N memories"
- Step 4: Only memories whose content contains "espresso" are listed; the counter updates; non-matching memories are hidden
- Step 6/7: Only memories with `kind = preference` are listed; each card shows a `preference` outline badge
- Step 9: The list is empty; the memories empty-state placeholder ("No memories yet. Agents using the memory capability will populate this store as they learn.") is shown; the counter is `0 memories`
- Search and filter persist across each other while both are set (intersection of both filters)
