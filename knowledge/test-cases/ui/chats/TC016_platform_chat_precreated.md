---
type: Test Case
title: "TC016: Chat - Permanent Conversation"
description: "Verify that permanent Chat is unique per user and organization and cannot be removed or reassigned."
tags:
  - everruns
  - test-case
  - ui
  - chats
---
# TC016: Chat - Permanent Conversation

## Description

Verify that permanent Chat is unique per user and organization and cannot be removed or reassigned.

## Preconditions

- Canonical DB-backed agent stack running
- User logged in to an organization with built-ins provisioned

## Steps

1. Finish onboarding and open **Chat**.
2. Reload twice and open the same organization in a second browser tab.
3. Confirm all entries resolve the same conversation and that it uses the managed Agent on Generic.
4. Confirm rename, pin/unpin, archive, delete, and counterpart selection are absent.
5. Attempt rename, unpin, archive, delete, or ownership reassignment through the session API.
6. Switch organizations and return; repeat reloads in both organizations.
7. Send a message and confirm the conversation remains usable.

## Expected Result

- Exactly one permanent conversation exists per user and organization, including concurrent entry.
- The direct sidebar entry and header read **Chat**; no redundant Platform Chat child row exists.
- Protected API mutations return 4xx; the conversation and its history remain available.
- Each organization keeps its own conversation; requests retain the selected organization header.
