# TC016: Chats - Platform Chat Precreated and Pinned

## Description

Verify that a user entering the application for the first time already has a Platform Chat
thread in Chats, that it is pinned, that onboarding ends in it, and that it is neither
duplicated on later entries nor recreated after the user archives it.

## Preconditions

- Server running (`just start-dev`)
- A user account that has never entered the application, or a fresh organisation for an
  existing user
- Built-in harnesses provisioned for the organisation (org setup completed)

## Test Data

None.

## Steps

1. Sign in as the new user and complete onboarding through the **Done** step
2. Press **Open Platform Chat** on the Done step and observe the route and the thread header
3. Navigate to `/chats` and observe the list and the sidebar under **Chats**
4. Reload the application twice and re-check `/chats`
5. Send one message in the thread and confirm it answers
6. Archive the thread from `/chats`, then reload the application and re-check `/chats`

## Expected Result

| Check | Expected |
|-------|----------|
| Precreated | A thread titled **Platform Chat** exists in `/chats` without the user creating one |
| Counterpart | The thread header shows the built-in **Platform Chat** harness, no agent |
| Pinned | The thread shows as pinned in `/chats` and carries the pin marker in the sidebar |
| Ordering | The pinned thread sorts above unpinned threads |
| Onboarding landing | **Open Platform Chat** on the Done step lands on `/chats/{sessionId}` for that thread |
| No duplicates | After reloads, exactly one Platform Chat thread exists |
| Usable | The thread answers a message like any other chat thread |
| Archive respected | After archiving and reloading, no new Platform Chat thread is created |
