---
type: Test Case
title: "TC010: Chat - Available Without Configuration"
description: "Verify that Chat works in a fresh organization without feature configuration or user-created Agents."
tags:
  - everruns
  - test-case
  - ui
  - chats
---
# TC010: Chat - Available Without Configuration

## Description

Verify that Chat works in a fresh organization without feature configuration or user-created Agents.

## Preconditions

- Canonical DB-backed agent stack running
- User logged in to an organization with built-ins provisioned

## Steps

1. Sign in to a fresh organization and open `/chats`.
2. Confirm **Chat**, **New chat**, and **All chats** are available in the sidebar.
3. Open `/chats/new`, send a message with an available model, and open the resulting side conversation.
4. Open `/chats/history` and confirm that conversation appears.

## Expected Result

- Managed Agent provisioning supplies the fixed counterpart without user setup.
- Chat has no feature opt-in or experimental badge.
- No Agent or harness selector appears; routes show useful loading/error states and no unhandled errors.
