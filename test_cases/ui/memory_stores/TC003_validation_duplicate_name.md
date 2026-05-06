# TC003: Duplicate Store Name Validation

## Description

Verify that creating a store with a name that already exists in the organization (case-insensitive) surfaces a non-disclosing error and does not create a duplicate.

## Preconditions

- UI running and user authenticated
- A store named `team-knowledge` already exists (from TC001)

## Test Data

| Field | Value |
|-------|-------|
| Store Name | TEAM-KNOWLEDGE |

## Steps

1. Navigate to **Memory** (`/memory-stores`)
2. Click **New Store**
3. Enter Name: `TEAM-KNOWLEDGE` (different casing of the existing store)
4. Click **Create**

## Expected Result

- Server returns a 409 Conflict and the UI shows a toast / inline error indicating the name is already in use
- The dialog stays open with the typed value preserved
- The store list does not gain a duplicate entry
- The existing `team-knowledge` card retains its previous memory count and ID
