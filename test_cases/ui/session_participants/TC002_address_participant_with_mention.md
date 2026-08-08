# TC002 Address participant with composer mention

## Description

Verify that a message can address an active guest agent through an accessible `@` mention in the
shared chat composer without changing default session routing.

## Preconditions

- A local (`AUTH_MODE=none`) or authenticated deployed Everruns UI is available
- A session with a host agent and at least two active guest agents is open
- Two guests share a display name; a third guest has a distinct display name

## Test Data

| Field | Value |
| --- | --- |
| Duplicate guest name | `Participant Specialist` |
| Distinct guest name | `Participant Researcher` |
| Addressed message | `Reply with exactly: mentioned reply` |
| Unaddressed message | `Reply with exactly: default reply` |

## Steps

1. Focus the composer, type `@`, and verify an autocomplete list appears above the composer with
   active guest agents only.
2. Type part of a participant name and verify the list filters. Type a non-matching query and verify
   the no-match state, then press Escape and verify the list closes.
3. Open the list again and use Arrow Up/Down followed by Enter to select `Participant Researcher`.
4. Verify the selected participant renders as a removable mention token inside the composer. Remove
   it, then select it again.
5. Enter the addressed message, send it, and verify the successful message request contains the
   selected guest's `addressed_participant_id`.
6. Verify the mention clears after the successful send. Send the unaddressed message and verify its
   request omits `addressed_participant_id`, preserving default routing.
7. Type `/` at the beginning of an empty composer and verify slash-command autocomplete still works.
8. Repeat steps 1 and 3 with the viewport at 390 px wide and verify the list and token remain usable
   without horizontal overflow.
9. Open `@Participant Specialist` and verify duplicate names have distinct participant suffixes.

## Expected Result

- `@` mention autocomplete is keyboard accessible, dismissible, filterable, and reports empty or
  no-match states.
- Only active addressable guest agents appear; duplicate display names are distinguishable.
- A selected mention is clear and removable, routes the next message to that participant, and clears
  after a successful send.
- Messages without a mention keep the existing default routing.
- Mention autocomplete coexists with slash commands and fits the mobile composer.
