# TC002: Create Memory - Required Name Validation

## Description

Verify that the New Memory dialog rejects a whitespace-only name with an inline field error and does not create a memory.

## Preconditions

- Server running (`just start-all`)
- User logged in with an org selected

## Test Data

| Field | Value |
|-------|-------|
| Memory name | `   ` (three spaces) |

## Steps

1. Navigate to `/memory`
2. Click **New Memory**
3. Type three spaces into the **Name** field
4. Click **Create Memory**
5. Observe the dialog
6. Click **Cancel**
7. Reload the memory list

## Expected Result

| Check | Expected |
|-------|----------|
| Inline error | `Name is required` is rendered below the Name input |
| Submit blocked | The dialog stays open and no network request to create a memory is made |
| Cancel | Dialog closes without creating a memory |
| No memory created | Reloaded memory list contains no new memory from this attempt |
