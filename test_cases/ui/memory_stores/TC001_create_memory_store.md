# TC001: Create Memory Store

## Description

Verify that a new org-scoped memory store can be created from the Memory page and is selected immediately after creation.

## Preconditions

- UI running (`just start-dev` or `just start-all`)
- User is authenticated and has access to an organization
- The current organization may have zero or more existing stores

## Test Data

| Field | Value |
|-------|-------|
| Store Name | team-knowledge |
| Make Default | unchecked |

## Steps

1. Sign in and navigate to the sidebar entry **Memory** (`/memory-stores`)
2. Click the **New Store** button in the page header
3. In the **New memory store** dialog, enter Name: `team-knowledge`
4. Leave "Make this the default store for the organization" **unchecked**
5. Click **Create**

## Expected Result

- Dialog closes after a successful create
- A new store card titled `team-knowledge` appears in the left store list
- The new store is auto-selected (its card has the active border)
- The card shows the store ID in the format `mst_<32-hex>` with a copy button
- The card shows `0 active memories`
- No `Default` badge is shown on the card (because the box was unchecked)
- The right pane shows the memories empty state: "No memories yet. Agents using the memory capability will populate this store as they learn."
