---
type: Test Case
title: "TC001: Chat - Permanent Landing and Fresh Side Draft"
description: "Verify that landing opens permanent Chat and New chat creates a fresh side conversation only on first send."
tags:
  - everruns
  - test-case
  - ui
  - chats
---
# TC001: Chat - Permanent Landing and Fresh Side Draft

## Description

Verify that landing opens permanent Chat and New chat creates a fresh side conversation only on first send.

## Preconditions

- Canonical DB-backed agent stack running
- User logged in to an organization with built-ins provisioned

## Steps

1. Navigate to `/`; confirm it opens `/chats` and the permanent Chat.
2. Confirm the fixed Platform Chat and its intro/starters render without an Agent or harness picker.
3. Open the sidebar's **New side chat**; inspect network requests and confirm no side session is created yet.
4. Send a message; confirm a distinct conversation opens at `/chats/{id}` on the same Agent and Generic.
5. Confirm it contains only the new message and reply, with no copied history or workspace files.
6. Open **All chats** and the sidebar; confirm the side conversation appears and the permanent Chat remains a direct navigation entry.

## Expected Result

- Landing opens the permanent conversation, never a picker or history list.
- First send creates one side session; retries reuse it if the message fails after creation.
- The standard composer, model controls, intro, and starters remain usable.
- **Open session** opens the read-only recording; Agent and harness selection remains in Playground.
