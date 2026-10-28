---
type: Test Case
title: "TC001: Customer tracing setup and routing"
description: "Verify protected destination setup, explicit multi-target routing, guarded Save and delivery feedback."
tags:
  - everruns
  - test-case
  - ui
---
# TC001: Customer tracing setup and routing

## Preconditions

- Canonical DB-backed stack running with AUTH_MODE=none, or an organization admin.
- A custom agent exists. Use synthetic test credentials and collector endpoints.

## Steps

1. Open Settings → Organization → Tracing. Add an OTLP destination and a Braintrust project.
2. Confirm both start Unassigned and metadata-only; reload and verify persisted non-secret configuration.
3. Edit defaults, select both, Discard, reopen and confirm neither was assigned. Select both and Save.
4. Open the agent's More → Tracing. Verify inherited effective destinations. Choose replacement, select one, and Save.
5. Reopen and select Add to defaults with an already-inherited destination; confirm it appears once in effective destinations.
6. Make an unsaved edit, update the same routing from another browser, then Save the first draft. Confirm conflict keeps the draft and requires review.
7. Open History and restore a previous policy. Confirm credentials and manual content settings are never in its snapshots.
8. Edit a destination. Confirm authentication is blank and preserved unless replaced; changing endpoint requires new authentication.
9. Send a synthetic test trace. Verify safe failure or receiver acceptance, including a health issue on delivery failure.
10. Pause a destination. Confirm Paused feedback and that it remains selected in routing. Delete while assigned and confirm rejection.
11. Check narrow viewport, keyboard navigation, light/dark modes and Ukrainian locale.

## Expected Result

Creation never assigns exports. Routing applies only on explicit Save, preserves revision guards,
and supports multiple destinations and providers. Setup remains a protected manual dialog.
Personal Chat is excluded. Acceptance describes the receiver response, not provider indexing.
Secrets never appear in responses, history, toasts or page-side Chat.
