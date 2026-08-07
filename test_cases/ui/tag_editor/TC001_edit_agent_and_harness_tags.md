# TC001 Edit Agent and Harness Tags

## Description

Verifies that agent and harness tags are edited as individual chips instead of a raw
comma-separated field.

## Preconditions

- Development stack is running.
- At least one editable agent and one editable harness exist.

## Test Data

| Field | Value |
|---|---|
| First tag | `support` |
| Pasted tags | `demo, customer-facing` |

## Steps

1. Open an editable agent and select Edit.
2. Enter the first tag and press Enter.
3. Confirm the tag appears as a removable chip and the text input clears.
4. Paste the comma-separated tags and confirm each value appears as its own chip.
5. Remove one chip and confirm the remaining chips are unchanged.
6. Save, reopen the agent editor, and confirm the remaining tags persist as chips.
7. Repeat steps 2–6 for an editable harness.
8. Open an archived agent or harness and confirm its tags cannot be changed.

## Expected Result

- Tags are displayed as individual chips with accessible remove controls.
- Enter and comma commit a typed tag; comma-separated paste creates multiple tags.
- Duplicate and empty tags are not added.
- Backspace from an empty input removes the final chip.
- Saved agent and harness tags persist and render as chips when reopened.
- Archived entities keep the tag editor read-only.
