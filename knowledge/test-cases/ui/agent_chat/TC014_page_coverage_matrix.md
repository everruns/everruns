---
type: Test Case
title: "TC014: Every page and new UI coverage"
description: "Verify every authenticated route, exposed editor field and relevant permission boundary against the page-aware Chat contract, including newly introduced interfaces."
tags:
  - ui
  - chat
  - adoption
  - permissions
---

# TC014: Every page and new UI coverage

## Description

Verify each supported UI surface has observable context, draft or protected-action
behavior. The maintained source inventory is
[`chat-page-coverage.json`](../../../../apps/ui/src/test-utils/chat-page-coverage.json).
An inventory entry is a coverage decision, not a passing manual execution record.

## Preconditions

- Running canonical authenticated stack with the tested source revision recorded.
- Separate owner, admin and member accounts in organization A, an unrelated
  account in organization B, and a platform operator for installation views.
- Disposable fixtures for the route families in the inventory; record resource
  IDs and starting revisions. Never use customer records for mutation scenarios.
- Chat configured with Muse 1.3 Contributor through OpenRouter.
- Exercise independent `chat_mode` and `chat_threads` Adoption settings in all
  four combinations. Enable each tested page's own feature separately.

## Test Data

| Fixture | Values |
| --- | --- |
| Collection | Northstar support; managed and connected files; lexical indexing |
| Draft | Northstar review, with an independent manual description edit |
| Translation | French Canadian display name and Ukrainian description |
| Secret sentinel | A disposable test key in a write-only input; never a real key |
| Conflict | A second authorized browser saves the same resource after editing starts |
| Display modes | Desktop split, 390 px mobile, light/dark, reduced motion |

## Steps

1. Enumerate every route in the inventory. For detail routes use the recorded
   fixture IDs. Follow redirects and record their final destinations; verify
   aliases do not construct another conversation.
2. For each view, open page-side Chat and request an explicit page read. Verify
   the route, current resource, tab, filters and displayed rows match the page.
   Change a filter, selected file or tab and read again without saving. Verify
   the new browser state appears, unrelated resources do not, and navigation
   alone creates no authored message.
3. For every editable form, use its returned patch schema to enumerate supported
   fields. Cross-check against the form's visible controls and the independent
   [field fixtures](../../../../apps/ui/src/__tests__/page-field-coverage.test.ts).
   Request one field change at a time, retaining the manual description. Verify
   Before/After and local Undo, and re-read the API to prove no write occurred.
4. Save an existing resource using its ordinary control. Re-read the API and
   history; verify the exact committed revision, attribution and diff. Navigate
   away and return; perform field Undo and Undo all. Verify persisted values and
   resulting history. Creation does not acquire a destructive deletion inverse.
5. Repeat relevant forms with member access, a revoked management policy and
   organization B. Denied or unavailable controls must not patch any field or
   persist a partial write. Verify the original fixture through an authorized
   independent read after each denial.
6. Repeat with an intervening manual edit, concurrent save, pending Save, failed
   Save, closed/reopened dialog, navigation and organization switch. Reject old
   bindings and stale revisions. Retain dirty text, block conflicting Save and
   recover through Discard. Retry tool-result delivery without a second patch.
7. Exercise the new Knowledge collection/account/source/search tabs and expanded
   file folders. Collapse an ancestor and verify descendants disappear from
   context. Confirm truncation is reported on large trees/results. Search
   excerpts, source config, raw failures, keys and file bodies are absent.
8. Edit Agent and Harness translations beside Chat. Verify draft and saved
   review/Undo preserve stable names and instructions. Reject unknown translation
   fields, duplicate normalized locales and oversized maps/strings atomically.
9. Stage interface language beside Chat: no language switch or device persistence
   before Save; Undo and Discard restore the draft. Save changes this device's
   interface language, preserving the runtime user's locale. With Adoption off,
   language switches immediately. Do not expect server entity history for a
   device preference.
10. Open trigger quick edit and manager notes. Verify each owns the adjacent
    binding, reads live fields and stages changes without saving. Notes require
    Edit first and preserve their own revision check. Their draft Undo is local;
    context-history entries are not resource-snapshot inverses.
11. Open protected history, sandbox, credential, file and approval surfaces over
    an editable page. Requests must not mutate the underlying form. Close the
    surface and verify the preceding binding is restored. Execution, upload,
    deletion, financial and credential actions keep their explicit manual controls.
12. Installation health requires platform authority, including for cached data.
    Deny a tenant admin, loading policy and policy error; return no global rows.
    Verify authorized status projections omit diagnostic payloads.
13. Repeat the four flag combinations and display modes. Preserve form and Chat
    drafts while opening/closing the split. Full Chat has one composer; history
    has none. New subchat creates an independent conversation on first send.
14. Include Tracing settings and the Agent routing drawer. Destination rows expose
    bounded status metadata without endpoints, credentials or raw delivery errors.
    Stage routing and destination selections; organization defaults keep their
    fixed routing mode. Loading/error states mask underlying editors even with
    cached data. Destination credentials and receiver actions remain manual.
15. Repeat interface-language draft/Save/Undo with every offered language, including
    regional English and Simplified Chinese. The registered language choices and
    the Chat patch schema must agree.
16. Save collection name and description together. Undo only the name and verify
    the description, indexing, embedding/search account and path rules remain at
    their latest saved values. Equivalent JSON key order must not disable Undo.
17. At a short desktop viewport and on mobile, open trigger quick edit with its
    schedule preview and a Chat draft diff. Scroll the dialog to Save and Cancel;
    both stay reachable within the page pane. Repeat with Chat workspace off.

18. Open New channel without a preselected type. Verify Chat has no editable
    channel draft while the type chooser is visible. Select a type and stage a
    field change; confirm the visible input updates without creating a channel.
    Click Change: the captured draft binding must fail reads and patches. Select
    the same type again and verify a fresh binding and default values. Repeat
    with a preselected type and a different type; ordinary Create stays manual.

## Expected Result

Every inventory route has a recorded result or a specific unmet prerequisite;
there are no silent omissions. Every supported field follows the draft and
permission contract. Protected controls retain their manual behavior and cannot
redirect an edit to an underlying form. Results distinguish catalog, unit,
fixture-browser and real-model manual evidence; a defined case alone is not a pass.
