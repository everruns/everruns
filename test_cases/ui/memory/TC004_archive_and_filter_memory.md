# TC004: Archive Memory and Toggle Archived Filter

## Description

Verify that a memory can be archived from the list, that archived memory are hidden by default, and that the archive filter toggle reveals them with a read-only `archived` badge and disabled write actions.

## Preconditions

- Server running (`just start-all`)
- User logged in
- At least one active memory named `tc004-archive-me` exists

## Test Data

| Field | Value |
|-------|-------|
| Memory name | `tc004-archive-me` |

## Steps

1. Navigate to `/memory`
2. Locate the `tc004-archive-me` card and click **Archive**
3. In the **Archive Memory** confirmation dialog, click **Archive**
4. Observe the memory list
5. Toggle the archive filter (the `Show archived` / archive filter control) to include archived items
6. Locate the `tc004-archive-me` card again
7. Click **Open** to navigate to the detail page

## Expected Result

| Check | Expected |
|-------|----------|
| Confirm dialog | Confirmation dialog text references `tc004-archive-me` |
| Card hidden | After archiving and with archive filter off, the card no longer appears |
| Filter reveals card | After enabling archive filter, the card reappears with `archived` status badge |
| Edit disabled | The card's **Edit** button is disabled |
| Archive button hidden | The **Archive** button is no longer rendered on the card |
| Detail page status | Detail page shows `archived` badge and an `Archived` activity timestamp |
| Detail Edit disabled | Detail page **Edit** button is disabled |
| Detail no Archive | Detail page does not render an **Archive** button |
