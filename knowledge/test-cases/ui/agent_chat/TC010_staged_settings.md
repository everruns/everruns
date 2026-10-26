---
type: Test Case
title: "TC010: Chat stages settings before Save"
description: "Verify Organization, model and search defaults stage safely under Adoption, preserve conflicts and retain legacy autosave when disabled."
tags: [ui, test-case, platform-chat, settings]
---

# TC010: Chat stages settings before Save

## Description

Verify the settings reviewed beside Chat are the settings saved through ordinary APIs.

## Preconditions

- Running stack, organization administrator, Chat mode adopted; a second browser on the same organization.
- Real chat model configured. Record organization ID and saved settings through the API.

## Test Data

| Field | Value |
| --- | --- |
| Organization name | Northstar Review |
| Model choices | Two available enabled models |
| Search results | 6 |

## Steps

1. Open Organization with Chat beside it. Change the name manually, ask Chat to
   change a supported default, and read back the draft. Verify both edits appear.
   Re-read the API before Save: neither edit may have persisted.
2. Use Undo and Discard, then repeat and use Save. Confirm exactly the reviewed
   fields persist and remain after reload. Repeat with Model defaults and Web
   search defaults; save each named form independently.
3. While a settings draft is dirty, change the same saved settings from the second
   browser. Refetch: keep the dirty form, show the conflict and disable Save and
   patches. Discard must load the new values. With a clean form, refetch must update it.
4. Force a failed save by temporarily taking the local API offline. The draft must
   remain editable and retryable; restoring the API and saving must persist it once.
5. Remove manage permission while a form is open. New patches and Save must fail;
   re-read the API to verify unchanged state. Repeat a stale revision and organization switch.
6. Disable Chat mode through the manual feature control. Verify the old shell and
   Organization/default autosave behavior; no new Save/Discard controls remain.
7. Repeat at desktop and narrow pane widths, dark mode and reduced motion. Settings
   navigation must fit the pane, controls remain reachable and Back to app preserves drafts.

## Expected Result

No Chat patch or adopted settings onChange writes to the server. Only the person's
Save persists the reviewed draft. Conflicts, failure and permission changes preserve
newer work; feature-off autosave remains functional.
