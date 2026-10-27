---
type: Test Case
title: "TC005: Page and organization binding"
description: "Verify that pending Chat tools cannot edit a different Agent, organization, or browser's form."
tags: [ui, test-case, agents, platform-chat]
---

# TC005: Page and organization binding

## Preconditions

- Two disposable editable Agents, A and B, with distinct instruction sentences.
- Two test organizations and a real model; Chat workspace enabled where needed.
- Browser trace available to identify pending page tool requests.

## Steps

1. On A, ask Chat for a draft edit. Once a draft read/request is in flight,
   navigate to B before the patch executes. Confirm B's form and saved definition
   remain unchanged; the old binding must fail or remain owned by its original page.
2. Send a new authored message from B asking for an edit. Confirm its context tag
   and tool binding identify B, and the fresh patch changes only B's draft.
3. Start another page request, then switch organizations. Confirm the conversation,
   composer, page registrations and change markers reset for the new organization.
   The old request must not alter any form or entity there.
4. Open the same organization and Chat in another browser context. While the first
   browser owns a pending page request, inspect the second. It must not execute
   the first browser's tools against its own form.
5. Close the originating browser while a page request is pending. Confirm normal
   client-tool timeout/cancellation resolves the turn; a reopened browser cannot
   reuse an expired binding. Send a fresh message to resume page editing.

## Expected result

- A tool requires the binding from its correlated authored message and the
  currently mounted adapter. Navigation alone creates no new authored turn.
- Wrong-page and wrong-organization callbacks cannot modify or save another Agent.
- Cross-browser event delivery does not duplicate draft execution.
- A new message from the intended page obtains a fresh, usable context.
