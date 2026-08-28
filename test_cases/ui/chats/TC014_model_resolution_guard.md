# TC014: Chats - Model Resolution Guard

## Description

Verify that Chat does not admit a turn without a resolvable model and can send a later message once
an explicit model becomes available.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- A chat thread exists
- No enabled chat model or resolvable default model exists

## Test Data

| State | User Message |
|-------|--------------|
| No model | `this must not create a turn` |
| Explicit model | `Reply with exactly: ready` |

## Steps

1. Open the existing chat thread with no resolvable model.
2. Confirm the composer explains that a model must be chosen or configured.
3. Confirm the message box, the attachment button, and voice (when enabled) are all disabled, while
   the model menu stays usable.
4. Attempt to type the no-model message and press Enter; confirm nothing is entered, Send stays
   disabled, and no turn event is created.
5. Enable a valid chat model and select it explicitly in the composer.
6. Send the explicit-model message in the same thread.
7. Wait for the response and inspect the session lifecycle.

## Expected Result

| Check | Expected |
|-------|----------|
| Missing-model guidance | The composer identifies the default model as unavailable and explains how to resolve it |
| Locked composer | Message box, attachment button, and voice are disabled while no model is resolvable |
| Escape hatch | The model menu stays enabled so a model can still be chosen |
| Keyboard submit | Enter does not submit while no model is resolvable |
| Send control | Send remains disabled while no model is resolvable |
| No dead turn | No input, `turn.started`, or active-session lifecycle is created by the blocked attempt |
| Explicit model | Selecting a valid model re-enables the composer and submission |
| Follow-up | The later message completes normally in the same thread |
| Terminal lifecycle | The valid turn emits `turn.completed` and `session.idled`; the session ends idle |
