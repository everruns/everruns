---
type: Test Case
title: "TC013: Shared saved review and guarded Undo"
description: "Verify Before/After, guarded Save and field-level saved Undo across supported resource editors, navigation and private settings."
tags: [ui, test-case, platform-chat, history, forms, permissions]
---

# TC013: Shared saved review and guarded Undo

## Description

Exercise the same review flow on every supported editor, including settings and
nested dialogs. Use the mapping tests as the current adapter inventory rather
than inferring write access from a page title.

## Preconditions

- Adoption Chat mode enabled in a disposable organization, a second browser/session
  for concurrent edits, and owner/admin plus read-only users.
- Real configured model for Chat turns. Use the requested Muse 1.3 Contributor
  through OpenRouter when available; record its exact model ID in the run evidence.
- Fictional fixtures; no private operational rows or real credentials in prompts.
- Save fixture IDs and initial values. Scheduled tasks remain disabled throughout.

## Test Data

| Field | Value |
| --- | --- |
| Initial description | Original Northstar description |
| Chat description | Reviewed Northstar description |
| Manual field | Keep this later manual edit |
| Fixture credential | browser-only-current-fixture |

## Steps

1. For Agents, Harnesses, Capabilities, Virtual Users, Observers, Evals and
   cases, Models, Providers, MCP metadata, Knowledge collections/sources, Memory, Sandbox Template metadata, Channels/Triggers and
   Schedules, wherever an existing editor is available: open a writable fixture, ask Chat to read its current form,
   patch two supported fields, and inspect Before/After. Undo one field, then Undo
   the remaining draft. No server value changes until normal Save. On read-only
   detail pages such as Skills, separately request a saved-resource update and
   verify its saved diff/Undo; a failed draft patch must never trigger that save.
2. Repeat the patch and Save. Verify the normal saved API values and a saved review
   with exact Before/After. Undo one saved field and verify the other remains saved;
   Undo all remaining fields and verify the original values. Review History after
   reload; temporary receipts may reset, server revisions must remain available.
3. Repeat on Organization, main/small/decision defaults, Web search, outbound
   allowlist, Account, organization Profile and an organization account name. A name-only account save
   must not require credential rotation. Account history must be unavailable to
   another member or tenant administrator, excluded from organization history,
   included in account export and erased with account deletion. A member can review and undo their own organization Profile fields without management permission, but cannot change lifecycle or reach another member’s self-service profile.
4. Leave a saved review, navigate to another page, and Undo it there. A dirty draft
   on the resource blocks saved Undo. Switch organization: discard the old receipt
   scope; late reads, saves and undo callbacks must not affect the new tenant.
5. Start editing in browser A. Commit another edit in B, including an API write
   while Chat is closed. A keeps its draft and refuses stale Save. For a saved
   receipt, a later write in B refuses Undo without changing B's values. Include
   a foreign save followed by Chat saves between polling intervals: Undo may
   reach only the uninterrupted Chat changes after the foreign save.
6. Disconnect history after a successful Save. Show "saved successfully" plus
   review recovery; Reload history rebuilds the review without a second mutation.
   Revoke manage permission: Undo fails with a visible error and keeps the receipt.
7. Open model selection, edit several booleans and refetch the catalog. Apply must
   compare picker-open values. Force one stale model: all writes and child history
   roll back. After a successful batch, stale one member and Undo all: no member
   changes. Per-model Undo restores only that model and preserves unrelated names.
8. Rotate a fixture provider header/channel token after capturing a baseline.
   Undo public diagnostics/configuration: current header names/values, channel
   auth/approval settings and credentials must survive unchanged. Sandbox metadata
   Undo preserves the current execution spec. Inspect history and tool results for
   fixture secret absence. Never paste an actual credential into Chat.
9. On the shared PostgreSQL store, fail an inverse after its schedule write: both
   schedule and history roll back. Nullable description/limits can return to unset.
   An external schedule store or remote MCP subscription inverse must refuse before
   mutation and explain the direct-review boundary.
10. Verify Creation, invitation, payment, token issuance and enqueue retain their
    existing submit actions. No saved Undo may delete a creation or reverse these
    effects. Adoption off retains the previous UI. Repeat review in dark mode,
    narrow pane/mobile layout, keyboard navigation and reduced motion.

## Expected Result

Supported editors share local review and guarded persisted Undo. Conflicts and
permission failures preserve drafts and receipts. Inverses preserve current
credentials, authority and unrelated fields. No partial model batch or PostgreSQL
schedule inverse commits. Read-only pages and irreversible actions retain explicit
manual controls.
