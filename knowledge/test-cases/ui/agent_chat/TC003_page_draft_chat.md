---
type: Test Case
title: "TC003: Chat edits the live Agent draft"
description: "Verify that adjacent Chat reads unsaved Agent form values, stages version-checked edits, and preserves manual work through Undo, Save, and Discard."
tags:
  - ui
  - test-case
  - agents
  - platform-chat
---

# TC003: Chat edits the live Agent draft

## Preconditions

- Canonical local stack running, health checked with its actual auth mode.
- Signed-in operator can manage Agents; an editable test Agent exists.
- Organization has adopted Chat workspace; Platform Chat has a real model configured.

## Steps

1. Create a disposable returns-support Agent. Save instructions to help with
   returns, ask for the order number, and check eligibility before promising a refund.
2. Edit Instructions to add `For damaged items, ask for a photo.` Do not save.
3. Open Chat beside the Agent. Ask: `Add a 30-day return window. Keep my damaged-item
   photo rule. Use short bullet points and leave the result in the page draft.`
4. Wait for the real model turn to complete. Inspect the live form, its gold draft
   markers, and the saved Agent API. Record the `read_page_draft` and
   `patch_page_draft` results from the session trace.
5. Use the Instructions draft Undo. Confirm the exact preceding manual prompt,
   including the photo rule, returns. The saved Agent must still be unchanged.
6. Ask Chat to reapply the change to the current draft. Review the form, then
   use Save changes. Reload the Agent and verify the 30-day window and photo rule.
7. Ask for another draft-only change, use Discard, and reload. Verify the last
   saved instructions remain. Check [TC004](TC004_draft_conflicts_and_undo.md)
   for overlapping manual edits and Undo conflicts.

8. Delay the first tool-results HTTP response while allowing session events to
   continue. Let the next page-tool request arrive before releasing that response.
   Confirm the next tool runs when delivery settles, without another message,
   a duplicate patch, or an automatic retry of a failed result submission.

## Expected result

- Live form values, including the unsaved sentence, appear in the read tool result.
- A patch changes the actual form, shows draft attribution, and makes no Agent API write.
- Chat says the edit is unsaved; Save is required to persist it.
- Undo restores the preceding manual draft; later manual edits cannot be overwritten.
- Save uses the normal validation and API; Discard removes the unsaved Chat edit.
- Model replies distinguish staged edits from saved changes.
- Use a real model for this case; a simulated response is not evidence of the
  complete user workflow. Shortening waiting time in a recording must be disclosed.

Run this scenario with a separate worker as well as an embedded host when that
deployment is supported. Confirm the model can call both page tools after the
server-to-worker transport; context metadata alone does not prove tool delivery.
