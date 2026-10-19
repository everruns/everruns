---
type: Test Case
title: "TC001: Session Storage - Secret Lifecycle and Isolation"
description: "Verify session secret creation, replacement, metadata-only listing, deletion, validation, and tenant isolation through the supported API."
tags:
  - everruns
  - test-case
  - api
  - session-storage
---
# TC001: Session Storage - Secret Lifecycle and Isolation

## Description

Verify session secret creation, replacement, metadata-only listing, deletion, validation, and tenant isolation through the supported API.

## Preconditions

- Canonical authenticated stack with private encryption configuration.
- Two disposable sessions in Org A and one in non-member Org B; record original metadata.
- Account A can manage its sessions. API requests include its credential and explicit org; no fallback cookies.

## Test Data

| Field | Value |
| --- | --- |
| Secret name | `MANUAL_SECRET_<run-id>` |
| Values | Two unique disposable strings, kept out of captured evidence |
| Invalid variants | Blank name, empty value, mixed valid/invalid batch |

## Steps

1. Use `PUT /api/v1/sessions/<session-a>/storage/secrets` with `{"secrets":{"<name>":"<value-one>"}}`.
2. List secrets through the same session path. Inspect the body for names/timestamps and verify it contains no value.
3. Replace the same name with value two; list again and verify there is one metadata entry for the name.
4. List the second session in Org A and verify this secret is absent there.
5. Try the invalid variants, including a new valid name plus an invalid name in one batch. Re-list and verify no partial batch write.
6. Repeat a list, write, and delete against Org B's session while selected in Org A. Re-read it using the authorized fixture.
7. Delete the secret through `DELETE /api/v1/sessions/<session-a>/storage/secrets/<name>` and list again.
8. Open the session Workspace view and verify it exposes metadata only, with no secret write controls.

## Expected Result

- Create and replace succeed; replacement does not duplicate the entry. Lists and Workspace never reveal values.
- Invalid batches fail validation before persisting any member of the batch.
- The secret belongs only to its session. Cross-tenant requests return 404 and leave the foreign session unchanged.
- Delete removes the metadata entry. No browser secret-edit form is required: session recordings are read-only.

## Source Contract

[Session storage commands](../../../../crates/server/src/domains/session_storage/commands/mod.rs) and [Workspace UI case](../../ui/session_files/TC001_read_only_workspace_and_storage.md).
