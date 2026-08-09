# TC001: Chats - Landing Route and New Thread

## Description

Verify that Chats is the app's landing route, that the empty state starts a thread rather than
spinning, and that a new thread opens on the thread surface bound to the agent that was picked.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- Feature flag `global_chat` enabled (`FEATURE_GLOBAL_CHAT=true`)
- At least one agent exists, or the built-in Platform Chat harness is available

## Test Data

None.

## Steps

1. Navigate to `/`
2. Observe the route landed on and the page content
3. If no threads exist yet, pick an agent in the empty state and press **Start chat**;
   otherwise open `/chats/new`, pick an agent, and press **Start chat**
4. Observe the thread surface, then send one message
5. Return to `/chats` and observe the list, then check the sidebar under **Chats**

## Expected Result

| Check | Expected |
|-------|----------|
| Landing | `/` redirects to `/chats` |
| Empty state | Shows "No chats yet" with an agent picker and **Start chat** — never a bare spinner |
| Thread created | `POST /v1/sessions` returns 200/201 with the picked agent, and the browser lands on `/chats/{sessionId}` |
| Thread header | Shows the agent avatar, the thread title, **Share**, and **Open session** |
| Composer | Placeholder names the bound agent; the model control shows the model in use |
| Agent binding | Shown as text, with no control to change the agent on an existing thread |
| Thread list | The new thread appears in `/chats` and under **Chats** in the sidebar |
| Open session | **Open session** navigates to `/sessions/{sessionId}` |
