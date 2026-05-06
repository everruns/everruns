# TC002: Create Default Memory Store

## Description

Verify that creating a store with "Make this the default store" checked sets it as the org default and clears the default flag from any previously default store.

## Preconditions

- UI running and user authenticated
- TC001 has been executed so at least one non-default store (`team-knowledge`) exists
- Optionally another store may already be marked default

## Test Data

| Field | Value |
|-------|-------|
| Store Name | org-default |
| Make Default | checked |

## Steps

1. Navigate to **Memory** (`/memory-stores`)
2. Note which store currently shows the **Default** badge (if any)
3. Click **New Store**
4. Enter Name: `org-default`
5. Check "Make this the default store for the organization"
6. Click **Create**
7. Reload the page

## Expected Result

- After step 6, the dialog closes and the new `org-default` card appears with a **Default** badge (star icon)
- The previously default store no longer shows the **Default** badge
- After step 7 (reload), `org-default` still shows the **Default** badge — i.e. the default flag is persisted server-side
- When the page reloads with no explicit selection, the default store is the one shown selected by default
