---
type: Test Case
title: "TC006: Draft permissions and saved-definition conflicts"
description: "Verify read-only Agent drafts and concurrent saved changes preserve the page's normal authorization and Save protections."
tags: [ui, test-case, agents, platform-chat]
---

# TC006: Draft permissions and saved-definition conflicts

## Preconditions

- Chat workspace and a real model enabled.
- Built-in Platform Chat Agent and an editable disposable Agent available.
- For the role check, authenticated manager and viewer accounts in a full-auth stack.

## Steps

1. Open the built-in Platform Chat Agent beside Chat. Ask to edit its instructions.
   Confirm its draft is read-only, no form patch succeeds, and the saved definition
   is unchanged. Repeat with an archived disposable Agent.
2. As a viewer lacking Agent management permission, ask to edit a readable Agent.
   Confirm the browser adapter refuses writes and no server save is used instead.
3. As a manager, make a dirty manual draft on the disposable Agent. In another
   authorized context, update its saved instructions. Wait for history/query refresh.
4. Confirm the first page retains its manual text, reports the saved-definition
   conflict, and disables Save. Dismiss change markers; Save must stay blocked.
5. Ask Chat to patch the conflicting draft. Confirm the adapter rejects the patch.
6. Use Discard. Verify the latest saved definition loads and editing can start again.
7. Start Save on an editable draft while a Chat request is in flight. Confirm a
   patch cannot change the form during its saving state.

## Expected result

- Built-in, archived, unauthorized, saving and conflicting drafts cannot be patched.
- Readability does not imply permission to edit. Existing server authorization
  remains authoritative for Save.
- A concurrent saved change never silently replaces the dirty draft or lets an
  older form overwrite the refreshed saved definition.
- Run role checks only with actual role-bound accounts; auth-none is not evidence
  of viewer restrictions.
