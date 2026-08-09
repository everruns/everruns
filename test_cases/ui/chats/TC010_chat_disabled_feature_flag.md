# TC010: Chats - Disabled Feature Flag

## Description

Verify that every Chats route explains itself when the `global_chat` feature flag is off, and that
the sidebar hides the Chats entry. A disabled flag must never produce a blank page (EVE-713).

## Preconditions

- Server running (`just start-dev`)
- User logged in
- Feature flag `global_chat` **disabled** (`FEATURE_GLOBAL_CHAT` unset or `false`)

## Test Data

None.

## Steps

1. Ensure `global_chat` feature flag is disabled
2. Check the sidebar for the "Chats" entry and the thread list
3. Navigate to `/` and observe the page content
4. Navigate directly to `/chats/new` and to `/chats/{any-session-id}`

## Expected Result

| Check | Expected |
|-------|----------|
| Sidebar | No "Chats" entry and no thread list |
| `/` | Redirects to `/chats`, which renders "Chats is not enabled" — not a blank page or a spinner |
| Escape hatch | A "Go to Sessions" link is offered |
| `/chats/new` | Same disabled notice |
| `/chats/{id}` | Same disabled notice; no `GET /v1/sessions/{id}` call is made |
| No errors | No console errors or unhandled exceptions |
