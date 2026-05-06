# TC002: Create Volume - Required Name Validation

## Description

Verify that the New Volume dialog rejects an empty or whitespace-only name and surfaces an inline field error without submitting the form.

## Preconditions

- Server running (`just start-all`)
- User logged in with an org selected

## Test Data

| Field | Value |
|-------|-------|
| Volume name (attempt 1) | (empty) |
| Volume name (attempt 2) | `   ` (whitespace only) |

## Steps

1. Navigate to `/volumes`
2. Click **New Volume**
3. Leave the **Name** field empty and click **Create Volume**
4. Observe the dialog
5. Type three spaces into the **Name** field and click **Create Volume**
6. Observe the dialog
7. Click **Cancel**

## Expected Result

| Check | Expected |
|-------|----------|
| Empty name rejected | Browser/HTML5 required validation prevents submit, OR an inline `Name is required` error renders |
| Whitespace name rejected | Inline `Name is required` error renders below the Name input; dialog stays open |
| No volume created | Returning to the list shows no new volume |
| Cancel | Dialog closes without creating any volume |
