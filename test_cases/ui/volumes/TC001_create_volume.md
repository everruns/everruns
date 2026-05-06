# TC001: Create Volume

## Description

Verify that a new workspace volume can be created from the Volumes page and appears in the list with `active` status and a `vol_` prefixed ID.

## Preconditions

- Server running (`just start-all`)
- User logged in with an org selected

## Test Data

| Field | Value |
|-------|-------|
| Volume name | `tc001-research` |
| Description | `Test volume for TC001` |

## Steps

1. Navigate to `/volumes` (sidebar entry "Volumes")
2. Click the **New Volume** button in the page header
3. In the dialog, enter `tc001-research` in the **Name** field
4. Enter `Test volume for TC001` in the **Description** field
5. Click **Create Volume**
6. Observe the volume list

## Expected Result

| Check | Expected |
|-------|----------|
| Dialog closes | Dialog closes without error after submit |
| Card appears | A card titled `tc001-research` appears in the grid |
| Status badge | Card shows an `active` status badge |
| ID format | Card ID matches `vol_` followed by 32 hex characters |
| Description | Card displays `Test volume for TC001` |
| Created timestamp | Card shows a recent "Created" relative time |

## Cleanup

- Archive the volume via the **Archive** button on the card to leave a clean list.
