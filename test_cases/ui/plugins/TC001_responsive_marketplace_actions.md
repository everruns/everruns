# Responsive marketplace card actions

## Description

Verify that marketplace card actions stay within the card and adapt to the card's available width.

## Preconditions

- Everruns is running in development mode with authentication disabled.
- The default Everruns marketplace is present.

## Test Data

| Field | Value |
|---|---|
| Route | `/plugins` |
| Wide viewport | 1440 × 1000 |
| Narrow-card viewport | 1024 × 800 |

## Steps

1. Open `/plugins` and select the **Marketplaces** tab at the wide viewport.
2. Confirm Browse, Sync now, Disable, and Remove are visible within the marketplace card.
3. Resize to the narrow-card viewport.
4. Confirm Browse remains visible, an overflow button labeled **More marketplace actions** appears,
   and the page has no horizontal overflow.
5. Open the overflow menu and confirm it contains Sync now, Disable, and Remove.
6. Select Disable, reopen the menu, and confirm the action changes to Enable.
7. Select Enable and return to the wide viewport.

## Expected Result

- The full action row is visible when the card is wide enough.
- Compact cards show Browse plus an accessible overflow menu without horizontal overflow.
- Menu actions retain the same enabled, disabled, and destructive behavior as their expanded
  counterparts.
