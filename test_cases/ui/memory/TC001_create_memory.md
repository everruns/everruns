# TC001: Create Memory

## Description

Verify that a new workspace memory can be created from the Memory page and appears in the list with `active` status and a `mem_` prefixed ID.

## Preconditions

- Server running (`just start-all`)
- User logged in with an org selected

## Test Data

| Field | Value |
|-------|-------|
| Memory name | `tc001-research` |
| Description | `Test memory for TC001` |

## Steps

1. Navigate to `/memory` (sidebar entry "Memory")
2. Click the **New Memory** button in the page header
3. In the dialog, enter `tc001-research` in the **Name** field
4. Enter `Test memory for TC001` in the **Description** field
5. Click **Create Memory**
6. Observe the memory list

## Expected Result

| Check | Expected |
|-------|----------|
| Dialog closes | Dialog closes without error after submit |
| Card appears | A card titled `tc001-research` appears in the grid |
| Status badge | Card shows an `active` status badge |
| ID format | Card ID matches `mem_` followed by 32 hex characters |
| Description | Card displays `Test memory for TC001` |
| Created timestamp | Card shows a recent "Created" relative time |

## Cleanup

- Archive the memory via the **Archive** button on the card to leave a clean list.
