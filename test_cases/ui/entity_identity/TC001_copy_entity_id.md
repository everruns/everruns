# TC001: Copy an entity ID from the canonical identity line

## Description

Verifies that entity cards and detail headers keep the readable name primary while exposing a
compact, accessible copy-ID action with the complete API-facing identifier.

## Preconditions

- Development stack running in auth mode `none`.
- At least one agent and one installed plugin are available.

## Test Data

| Field | Value |
|---|---|
| Agent | Any named agent |
| Installed plugin | A plugin whose capability reference includes the `plugin:` namespace |
| Desktop viewport | 1440 x 900 |
| Mobile viewport | 390 x 844 |

## Steps

1. Open `/agents` at the desktop viewport.
2. Verify each agent card shows its readable name followed immediately by a compact `#` button and
   does not show a separate raw ID line.
3. Focus the `#` button using the keyboard and verify the tooltip and accessible name are
   `Copy ID: <full ID>` with no truncation.
4. Activate the button and verify the clipboard contains the exact ID and the control announces
   copied feedback.
5. Open the agent detail page and repeat the identity, tooltip, clipboard, and feedback checks in
   the page masthead.
6. Open `/plugins`, locate an installed plugin, and verify its readable name and `#` button share
   one identity line without a duplicate raw capability reference.
7. Verify the plugin tooltip exposes the complete namespace-prefixed capability reference and the
   clipboard receives that exact value.
8. Repeat the agent-card and plugin-card layout checks at the mobile viewport.
9. Verify long names/IDs do not create horizontal overflow or collide with badges and actions.

## Expected Result

- Readable names remain primary at desktop and mobile widths.
- Every ID action has the complete `Copy ID: <full ID>` tooltip and accessible name.
- Keyboard activation copies the exact API-facing value and exposes consistent copied feedback.
- Namespace prefixes are preserved.
- No duplicate/raw long ID line crowds a card and no horizontal page overflow is introduced.
