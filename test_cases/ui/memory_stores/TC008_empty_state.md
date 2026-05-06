# TC008: Memory Page Empty State

## Description

Verify that visiting the Memory page in an org with zero memory stores shows the empty-state with a call-to-action that opens the create dialog, and creating a store from the empty state transitions the page into the populated layout.

## Preconditions

- UI running and user authenticated
- Active org has **no** memory stores yet (use a freshly created org, or sign in with a user whose default org has none)

## Test Data

| Field | Value |
|-------|-------|
| Store Name | first-store |

## Steps

1. Navigate to **Memory** (`/memory-stores`)
2. Observe the page contents
3. Click the empty-state **New Store** button
4. Enter Name: `first-store`
5. Click **Create**

## Expected Result

- Step 2: A centered empty state is shown with the brain icon, the heading "No memory stores", explanatory text mentioning that the default store is created on first agent use, and a **New Store** call-to-action button. The two-column layout (sidebar + memories pane) is **not** rendered.
- Step 5: After creation, the page transitions to the two-column layout. The `first-store` card is visible in the sidebar and selected. The right pane shows the "No memories yet…" memories empty state.
