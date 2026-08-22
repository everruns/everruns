# TC015: Chats - Archive Chat

## Description

Verify that a chat can be archived and restored from both the Chats list and the open thread, that
archived chats leave the default list and the sidebar, and that the **Show archived** filter brings
them back without destroying the thread.

## Preconditions

- DB-backed stack running
- User logged in
- At least two chat threads exist for the current user, one with a reply in its transcript

## Test Data

None.

## Steps

1. Open `/chats` and note the listed threads and the sidebar thread list
2. Press **Archive chat** on the row of the thread that has a transcript
3. Observe the Chats list and the sidebar
4. Open the **Filter** menu and enable **Show archived**
5. Open the archived thread from the list and confirm its transcript is intact
6. Press **Unarchive chat** in the thread header, then return to `/chats`
7. Disable **Show archived** and reload `/chats`

## Expected Result

| Check | Expected |
|-------|----------|
| List action | The archived row disappears from the default list; `PUT /v1/sessions/{id}/archive` returns 204 |
| Sidebar | The archived thread is gone from the sidebar list, and the **All chats** link is still offered |
| Filter | Enabling **Show archived** re-lists the thread, dimmed and badged **Archived**, below unarchived threads |
| Thread | Opening the archived thread loads its full transcript and header, with an **Archived** badge |
| Restore | **Unarchive chat** returns the thread to the default list and activity-time ordering |
| Persistence | Reloading `/chats` with the filter off shows the restored thread and hides any still-archived ones |
