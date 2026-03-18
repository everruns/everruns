# TC010: Global Chat - Disabled Feature Flag

## Description

Verify that the global chat page shows a disabled message when the `global_chat` feature flag is off, and that the sidebar hides or disables the chat entry.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- Feature flag `global_chat` **disabled** (`FEATURE_GLOBAL_CHAT` unset or `false`)

## Test Data

None.

## Steps

1. Ensure `global_chat` feature flag is disabled
2. Check the sidebar for the "Chat" entry
3. Navigate directly to `/chat`
4. Observe the page content

## Expected Result

| Check | Expected |
|-------|----------|
| Sidebar | "Chat" entry hidden or not present |
| `/chat` page | Shows "Global Chat is not enabled" message |
| Sub-message | Shows "This feature is currently disabled." |
| No session created | No `POST /v1/sessions/chat` call made |
| No errors | No console errors or unhandled exceptions |
