---
type: Test Case
title: "TC009: Chat edits shared page forms"
description: "Verify live staged edits, Undo, independent forms and stable dialogs beyond Agent definitions."
tags: [ui, test-case, platform-chat, forms]
---

# TC009: Chat edits shared page forms

## Preconditions

- TC008 setup; editable fictional records and permitted create routes.
- Record saved values before testing; keep all fixture data in the test organization.

## Steps

1. Repeat the draft flow on Harness create/detail, Virtual User create/detail,
   Observer create/edit, Eval creation, Model ID/display-name dialogs, declarative Capability create/edit, Knowledge
   collection/source/account forms, Memory and legacy Knowledge-index metadata,
   Skill creation, Sandbox-template metadata, and non-credential Channel text.
   Read the returned patch schema first: it defines that form's supported fields.
2. Make a manual draft edit. Ask Chat to change one other supported field and leave
   it unsaved. Verify the existing controls update, the manual edit survives, the
   notice names changed fields, and the saved record/API value remains unchanged.
3. Use draft Undo. Verify preceding values return, including prior manual edits.
   Patch again, edit that same field manually, then Undo: it must refuse to overwrite
   the later manual edit. Discard/Cancel must clear the draft and stale change markers.
4. Patch again, review and use the existing Save/Create action. Verify ordinary
   validation runs and persistence happens once through that action. Reload to confirm.
   Do not expect saved Agent-style inverse Undo on another resource.
5. On a Provider page, read all named forms. Change tracing only: details and advanced
   controls must remain intact. Mixed-form, unknown-field and oversized aggregate
   patches must fail without changing any form. Save tracing with its own Save action.
6. Manually type a fixture header/API-key value. Patch supported metadata or cache
   diagnostics. Verify the credential input is unchanged and absent from tool reads.
   MCP headers/environment, sandbox environment and channel auth must not be writable.
   A credential-bearing URL disables its form adapter; URL patches with credentials fail.
7. Open a Skill/Memory/Knowledge dialog with an unsaved draft, then open Chat. Focus
   and type in Chat: the dialog must stay mounted, contain the same draft and remain
   in the page pane. Close/Escape explicitly; the underlying page binding must return.
8. While Chat is thinking, change a field or navigate to another page/form. A stale
   patch must fail. Repeat by closing a dialog, replacing a record on the same route,
   and switching organizations. None may reuse the preceding binding.
9. Refresh a saved record while its draft is dirty. Keep the draft visible, block its
   Save and new patches, then Discard/Cancel and reopen to review the current record.
   Try a built-in, archived or unauthorized form: reads may succeed, patches must fail.
10. Repeat dialog and form flows at 1366 px, 390 px, dark mode and reduced motion.
    The page body stacks to its available pane width. Back to app restores mobile
    drafts; navigation menus, focus and explicit dismissal remain usable.
11. With an Observer containing an existing API scorer variant unsupported by its
    current editor, read its context and patch only its name. The scorer collection
    must remain readable, marked read-only and preserved. Changing the schema/form
    must invalidate the preceding adapter immediately.

## Expected result

Chat stages only allowlisted UI fields. No patch invokes a save callback or immediate
page action. Normal Save/Create applies reviewed drafts; bindings, validation, guarded
Undo and conflict checks preserve newer work and credentials.
