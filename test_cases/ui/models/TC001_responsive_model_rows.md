# Responsive model rows

## Description

Verify that the Models page remains readable and interactive when the app content area narrows.

## Preconditions

- Everruns is running in development mode with authentication disabled.
- At least one enabled model with profile capabilities is configured.

## Test Data

| Field | Value |
|---|---|
| Route | `/models` |
| Compact desktop viewport | 1024 × 900 |
| Tablet viewport | 768 × 900 |

## Steps

1. Open `/models` at the compact desktop viewport.
2. Confirm the provider rail stacks below the model list.
3. Confirm each model's identity, capability badges, health indicator, and actions remain within its
   bordered row.
4. Resize to the tablet viewport.
5. Confirm model metadata and actions wrap without clipping or horizontal page overflow.
6. Expand a model profile and confirm the detail grid remains readable.

## Expected Result

- Model rows adapt to their available width without clipping content.
- Every model action remains visible and usable.
- The page has no horizontal overflow at either viewport.
- The provider rail and expanded profile details stack into readable layouts.
