---
type: Test Case
title: "TC014: Chat - Platform Only and Playground Isolation"
description: "Verify that Chat uses the managed Agent exclusively and excludes Playground and other runs from history."
tags:
  - everruns
  - test-case
  - ui
  - chats
---
# TC014: Chat - Platform Only and Playground Isolation

## Description

Verify that Chat uses the managed Agent exclusively and excludes Playground and other runs from history.

## Preconditions

- Canonical DB-backed agent stack running
- User logged in to an organization with built-ins provisioned

## Test Data

One Playground conversation using Generic and a custom Agent, one ordinary API run,
and one platform side conversation.

## Steps

1. Create a Playground conversation and an ordinary run with a custom Agent.
2. Open `/chats/new`; confirm there is no counterpart selector or Start chat step.
3. Send a platform message and verify the managed Agent and Generic are fixed.
4. Open `/chats/history` and inspect recent sidebar conversations.
5. Open the custom run through `/chats/{id}` and confirm the recording redirect guidance.
6. Use **Test in Playground** on its recording; confirm the Agent is preselected in Playground.

## Expected Result

- History and the sidebar list only the user's platform side conversations.
- Permanent Chat, Playground conversations, and arbitrary Agent runs are absent from history.
- Exclusion happens before pagination: a full page and total describe side conversations only.
- Testing retains Agent/harness selection in Playground.
