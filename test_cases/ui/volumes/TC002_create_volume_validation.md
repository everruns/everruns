# TC002: Create Volume - Required Name Validation

## Description

Verify that the New Volume dialog rejects a whitespace-only name with an inline field error and does not create a volume.

## Preconditions

- Server running (`just start-all`)
- User logged in with an org selected

## Test Data

| Field | Value |
|-------|-------|
| Volume name | `   ` (three spaces) |

## Steps

1. Navigate to `/volumes`
2. Click **New Volume**
3. Type three spaces into the **Name** field
4. Click **Create Volume**
5. Observe the dialog
6. Click **Cancel**
7. Reload the volumes list

## Expected Result

| Check | Expected |
|-------|----------|
| Inline error | `Name is required` is rendered below the Name input |
| Submit blocked | The dialog stays open and no network request to create a volume is made |
| Cancel | Dialog closes without creating a volume |
| No volume created | Reloaded volumes list contains no new volume from this attempt |
