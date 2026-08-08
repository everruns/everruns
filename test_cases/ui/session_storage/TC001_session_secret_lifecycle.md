# TC001: Session Secret Lifecycle

## Description

Verify that the Session Storage UI can create, replace, list, and delete a
session-scoped encrypted secret without reading the value back.

## Preconditions

- Canonical local stack is running.
- A Session whose effective harness includes `session_storage` exists.
- Use disposable values only.

## Steps

1. Open the Session's **Advanced → Storage** page.
2. Enter a name and disposable value. Confirm the value input is masked and has
   password-manager/autocomplete protections.
3. Save and confirm both inputs clear. The list shows only the name, encrypted
   state, and timestamp.
4. Inspect the list API response and browser DOM; neither contains the value.
5. Choose **Replace**, enter a second disposable value, and save. Confirm it is
   cleared and not displayed.
6. Delete the secret and confirm the irreversible action. The name disappears.

## Expected Result

- Create/upsert, rotate, list metadata, and delete all succeed.
- Values are never prefilled or read back and leave client state after each
  submission.
- Scope copy explains that the value belongs only to this Session, is readable
  by `secret_store`, and is not the durable credential path for Agent Triggers.
