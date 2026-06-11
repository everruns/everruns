# TC006: Memory Capability Mount Editor

## Description

Verify that the **Memory** capability config editor (on an agent or harness) lets users add, configure, and remove memory mounts, and surfaces inline validation for invalid paths, duplicate paths, and overlapping paths.

## Preconditions

- Server running (`just start-all`)
- User logged in
- At least two active memory exist:
  - `tc006-research`
  - `tc006-team-memory`
- An editable agent or harness with capability configuration access

## Test Data

| Field | Value |
|-------|-------|
| Mount 1 memory | `tc006-research` |
| Mount 1 path (valid) | `/workspace/research` |
| Mount 1 mode | Read only |
| Mount 2 memory | `tc006-team-memory` |
| Mount 2 path (valid) | `/workspace/team-memory` |
| Mount 2 mode | Read / write |
| Invalid path | `/etc/passwd` |
| Overlapping path | `/workspace/research/docs` |

## Steps

1. Open an agent (or harness) edit page that exposes the capability list
2. Enable / open the **Memory** capability configuration
3. Click **Add mount**
4. In the new row, select memory `tc006-research`, leave the path empty, then enter `/etc/passwd`
5. Observe the path error
6. Change the path to `/workspace/research`, mode `Read only`
7. Click **Add mount** to add a second row
8. In the second row, select memory `tc006-team-memory`, set path to `/workspace/team-memory`, mode to `Read / write`
9. Save the agent / harness
10. Re-open the same edit page and observe both mount rows are persisted
11. Click **Add mount** to add a third row, select `tc006-research`, set path to `/workspace/research/docs`
12. Observe the overlap error
13. Change the third row path to `/workspace/team-memory` (duplicate of row 2)
14. Observe the duplicate error
15. Click the trash icon on the third row to remove it and save again

## Expected Result

| Check | Expected |
|-------|----------|
| Memory picker | Combobox lists active memory including `tc006-research` and `tc006-team-memory`; archived memory are not selectable |
| Invalid path error | Path `/etc/passwd` shows `Path must be /workspace or start with /workspace/` |
| Valid mount accepted | Path `/workspace/research` clears the error |
| Two mounts persist | After save and reload, both rows are present with the saved memory IDs, paths, and modes |
| Overlap error | Path `/workspace/research/docs` shows an overlap error referencing `/workspace/research` on both affected rows |
| Duplicate error | Path equal to an existing mount path shows a `Duplicate mount path` error on both rows |
| Remove mount | Trash icon removes the row immediately; saving persists the deletion |
| Empty state hint | If the org has no active memory, the editor shows "No active memory found in this org" instead of an empty mount list |
