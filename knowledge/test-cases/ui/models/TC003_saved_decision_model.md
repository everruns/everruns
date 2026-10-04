---
type: Test Case
title: "TC003: Saved decision model"
description: "Verify that Jev reuses a saved provider account, remains separate from chat models, and records its serving snapshot and usage."
tags:
  - everruns
  - test-case
  - ui
  - models
---
# TC003: Saved decision model

## Description

Verify that Jev reuses a saved provider account, remains separate from chat models, and records its serving snapshot and usage.

## Preconditions

- Canonical local stack or authenticated deployment, with experimental capabilities enabled.
- A configured OpenRouter account and a working chat model.

## Test Data

| Field | Value |
| --- | --- |
| Profile | Jev 1.13 |
| Content | The sky is blue on a clear day. |
| Primitives | Noul, choice, score |

## Steps

1. In Models, select Decisions and enable Jev 1.13 for the existing OpenRouter account. Inspect its profile and select it as the default decision model.
2. Inspect chat model selectors; Jev must be absent. Attempt to use its saved ID as an agent chat default through the API; expect rejection.
3. Add Jev Decisions to an agent and select that exact saved model in its capability settings.
4. Run a session asking for one evaluation with all three primitives.
5. Inspect the decision generation event and usage ledger: saved account, model profile, actual serving snapshot, token usage, and cost must be recorded.
6. Disable the saved model. Reopen its selector; the old selection must remain visible with repair guidance. Execute again; the call must fail without falling back to another account.
7. Repeat the model reference from another organization; expect rejection.

## Expected Result

- One existing account authenticates chat and decisions without a second credential entry.
- Service filters and selectors keep decision models out of chat and embedding choices.
- The tool returns validated calibrated results and records billing through the existing ledger.
- Unavailable or foreign bindings fail closed and remain repairable.
