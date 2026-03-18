# TC001: Global Chat - Page Loads

## Description

Verify that the global chat page loads successfully, creates a singleton session, and displays the chat interface.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- Feature flag `global_chat` enabled (`FEATURE_GLOBAL_CHAT=true`)

## Test Data

None.

## Steps

1. Navigate to `/chat` in the sidebar
2. Wait for the loading spinner to disappear
3. Observe the page header and chat panel

## Expected Result

| Check | Expected |
|-------|----------|
| Page header | Shows "Chat" with experimental badge |
| Chat panel | Renders with message input area |
| No error | No "Failed to load chat" message |
| Session created | `POST /v1/sessions/chat` returns 200/201 |
| Revisiting `/chat` | Same session reused (singleton) |
