---
type: Test Case
title: "TC011: Secondary forms and protected dialogs"
description: "Verify secondary editors stage supported fields, preserve manual secrets and invalidate old dialog bindings."
tags: [ui, test-case, platform-chat, forms, credentials]
---

# TC011: Secondary forms and protected dialogs

## Description

Verify the shared draft flow reaches secondary dialogs and editors without exposing credentials.

## Preconditions

- TC010 setup, fictional MCP server, schedule, eval and Virtual User in the test organization.
- Record fixture IDs and saved values; use a fixture secret, never a real credential in a prompt.

## Test Data

| Field | Value |
| --- | --- |
| MCP name | Northstar tools |
| Invitation | reviewer@example.test |
| Fixture secret | browser-only-fixture |

## Steps

1. Repeat manual edit → Chat edit → Undo → review → Save/Create on My agent
   experience, invitation, organization account name, provider connect metadata,
   model selection, personal/catalog MCP metadata, schedule create/edit, eval case,
   report builder and Playground setup. Read each patch schema before requesting edits.
   Inspect the saved API value before and after the manual submit action.
2. In mixed credential forms type the fixture secret manually. Read and patch the
   supported metadata: the secret must remain unchanged in its input and absent
   from every tool result. URL userinfo/query credentials must disable the adapter;
   attempted incoming credential URLs must fail without changing any field.
3. Open a credentials, header, tool approval or generated-token popup above a dirty
   page. Read context: it must identify manual controls and be read-only. A patch
   targeting the page beneath it must fail. Close the popup and verify the original
   page draft remains. Generated token values never enter the conversation.
4. In a secondary edit dialog, refetch identical saved data while dirty: retain
   the draft. Change the saved definition elsewhere and refetch: retain the draft,
   block Save, and allow Discard or Cancel/reopen to load current values. A clean
   dialog must follow remote values. Closing/reopening rejects captured old bindings.
5. On every Channel kind patch supported public configuration, including Voice and
   Slack. Verify browser sign-in, tokens, API auth methods and approval policy survive
   unchanged. Repeat on an existing GitHub trigger and preserve unsupported filter clauses.
6. On Enqueue Task patch only metadata. Queued and scheduled payloads stay manual and absent from
   tool reads. The queue must remain unchanged until the person presses Enqueue.
   Playground must create no session before Start; invitation must send no invite
   before the normal submit button.
7. Revoke manage permission while dialogs are open. They must stop offering draft
   writes; verify unchanged saved state. Repeat with Adoption off and narrow/mobile layout.

## Expected Result

All supported secondary forms reuse local drafts and existing submit validation.
Secrets and manual actions remain outside the adapter. Refresh, navigation and
permission changes cannot apply a delayed patch to another form or overwrite newer work.
