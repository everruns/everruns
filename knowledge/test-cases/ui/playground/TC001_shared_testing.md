---
type: Test Case
title: "TC001: Playground - Shared agent testing"
description: "Verify standard navigation, identity binding, shared inspection, and reuse of session views."
tags:
  - everruns
  - test-case
  - ui
  - playground
---
# TC001: Playground - Shared agent testing

## Description

Verify standard navigation, identity binding, shared inspection, and reuse of session views.

## Preconditions

- Canonical DB-backed stack running, with a configured chat model and a test agent.
- Organisation has an admin, a member, and two active end-user virtual users.
- No Playground feature-flag configuration is required.

## Test Data

| Field | Value |
|-------|-------|
| Message | Reply with exactly: Playground works |
| Title | Playground smoke test |

## Steps

1. Open Playground under Building. Start a new Playground chat; verify Talk as defaults to your linked virtual user.
2. As an admin, select another active end-user virtual user. Choose the test agent and start.
3. Send the test message. Verify streaming, final output, rename, and the fixed agent/user facts.
4. Use Open session and verify the same session identifier and transcript. Use Workspace to inspect the same run. Verify Trace and Pin chat are absent.
5. Sign in as another organisation member. Find the chat in Playground. Verify it is inspectable, but sending as someone else's user requires admin authority.
6. Filter the library by agent, virtual user, and title. Archive the chat; verify it moves into Archived and can be restored.
7. Verify Playground chats are absent from personal Chats and its sidebar thread list. Confirm a normal chat still sends successfully.
8. Use the chat breadcrumb to return to Playground. Open the same chat with the explicit Open chat row action. Follow the agent and virtual-user links from both the list and chat details; verify each opens the correct entity.
9. Verify list, setup, and detail use the standard page masthead, gold create action, and underline tabs in both themes and at a narrow viewport.

## Expected Result

- The selected identity is immutable, in-org, active, and an end user; service and foreign identities are rejected.
- Everyone in the organisation can view shared history. Another organisation cannot load it.
- Operator provenance is distinct from the simulated subject, with no private user credentials or memory exposed.
- Conversation rendering, attachments, model controls, workspace browsing, and recordings reuse the existing components.
