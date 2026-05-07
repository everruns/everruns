# TC005: Forget a Memory

## Description

Verify that the **Forget** action on a memory card deactivates the memory: it disappears from the default list, the store's active memory count decreases, and the memory remains visible only when including inactive entries.

## Preconditions

- UI running and user authenticated
- A store exists with at least 2 active memories (see TC004 for seeding)
- User has the `OrgSettingsManage` permission (the role used to sign in must be permitted to call `DELETE /v1/memory-stores/{store_id}/memories/{memory_id}`)

## Test Data

| Field | Value |
|-------|-------|
| Target memory | First card in the active list |

## Steps

1. Navigate to **Memory** and select the target store
2. Note the **active memory count** displayed on the store card (call it `N`)
3. Note the memories counter in the right pane (call it `M`)
4. Click the **Forget** button (trash icon) on the first memory card
5. Wait for the request to settle

## Expected Result

- The forgotten memory disappears from the list
- The right-pane memory counter decreases to `M - 1`
- The store card's active memory count decreases to `N - 1`
- A success toast or in-place state confirms forget succeeded
- Re-running the same forget call against an already-forgotten memory ID via the API returns 404 (existence is not re-disclosed)
- Calling `GET /v1/memory-stores/{store_id}/memories?include_inactive=true` returns the forgotten memory with `active: false` (and the UI would render the `forgotten` destructive badge if the memory were displayed)
