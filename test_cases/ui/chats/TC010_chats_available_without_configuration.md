# TC010: Chats - Available Without Configuration

## Description

Verify that every Chats route and the sidebar are available to a fresh organization without
feature configuration, including the app's landing redirect.

## Preconditions

- Server running (`just start-dev`)
- User logged in to a fresh organization
- At least one agent exists

## Test Data

One agent and one existing chat thread.

## Steps

1. Open Settings → Features and confirm Chats is not offered as an opt-in feature
2. Check the sidebar for the "Chats" entry and its recent thread list
3. Navigate to `/` and observe the resulting route and page content
4. Navigate directly to `/chats/new`
5. Start a new thread and open it at `/chats/{thread-id}`

## Expected Result

| Check | Expected |
|-------|----------|
| Settings → Features | No Chats opt-in appears |
| Sidebar | "Chats" is always present without an experimental badge; recent threads render |
| `/` | Redirects to `/chats`, which renders the list or usable new-chat empty state |
| `/chats/new` | Agent picker renders and can create a thread |
| `/chats/{id}` | The thread loads with its transcript and composer |
| No errors | No disabled notice, console errors, or unhandled exceptions |
