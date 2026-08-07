# Default model autosave

## Description

Verify that organization default-model changes save independently, remain scoped to the selected organization, and show contextual status and errors.

## Preconditions

- DB-backed stack with at least one enabled model
- User is an owner or admin of the selected organization
- Organization settings page is open

## Test Data

| Field | Value |
|---|---|
| Default model | Any enabled model other than the current default |

## Steps

1. Change Default Model and observe the Models status.
2. Confirm the PATCH payload contains only `default_model_id`.
3. Reload the page and confirm the selected model remains selected.
4. Change a Harness setting while a model save is pending and confirm each section reports its own status.
5. Trigger a model validation, authorization, or network failure.
6. Repeat at a mobile viewport.

## Expected Result

- The model change saves and persists after reload for only the selected organization.
- Models and Harnesses save independently during rapid or overlapping changes.
- Model errors render directly with Models in an accessible alert; Harness errors render with Harnesses.
- No detached page-level save error appears and the page has no mobile horizontal overflow.
