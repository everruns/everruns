# TC013: Chats - Pin Chat

## Description

Verify that a chat can be pinned or unpinned from both the Chats list and the open thread, and that
pinned chats stay ahead of more recently active unpinned chats.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- At least two chat threads exist for the current user

## Test Data

None.

## Steps

1. Open `/chats` and identify an older unpinned chat
2. Press its **Pin chat** button
3. Observe the Chats list and the sidebar
4. Open the pinned chat and press **Unpin chat** in the thread header
5. Return to `/chats`
6. Open a different unpinned chat, press **Pin chat** in the thread header, then return to `/chats`

## Expected Result

| Check | Expected |
|-------|----------|
| List action | The row action changes from **Pin chat** to **Unpin chat** after pinning |
| Pinned order | The pinned chat moves above every unpinned chat; activity time orders chats within each group |
| Sidebar | The pinned chat appears first among available threads and carries a pin indicator |
| Header action | The open thread exposes the same pin/unpin state and action as the list |
| Persistence | Reloading `/chats` preserves the pin state and ordering |
| Unpin | The chat returns to activity-time ordering after it is unpinned |
