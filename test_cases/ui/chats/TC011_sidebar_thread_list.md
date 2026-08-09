# TC011: Chats - Sidebar Thread List

## Description

Verify that the sidebar lists the most recently active threads under Chats, capped at five, and
that the list does not re-order under the cursor while the pointer is inside it.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- Feature flag `global_chat` enabled
- At least seven chat threads exist for the current user

## Test Data

None.

## Steps

1. Open `/chats` and note the order of threads
2. Check the sidebar under **Chats**
3. Hover the sidebar thread list and keep the pointer inside it
4. From another browser tab (or a second window), send a message in the thread that is currently
   last in the sidebar list, and wait for the reply
5. Keep the pointer inside the list for at least 30 seconds, then move it away

## Expected Result

| Check | Expected |
|-------|----------|
| Cap | At most five threads listed under **Chats** |
| Order | Most recently active thread first |
| Extra rows | A **New chat** row, and an **All chats** row linking to `/chats` |
| Frozen order | While the pointer is inside the list, no row changes position |
| Thaw | After the pointer leaves, the newly active thread moves to the top |
| Active row | The open thread's row is highlighted |
