# TC001: Created Organization Visible in List

## Description

Verifies that a newly created organization immediately appears in the user's organization list without requiring re-login or page refresh.

## Preconditions

- User is authenticated
- User has permission to create organizations

## Steps

1. Call `GET /v1/orgs` and note the current org count
2. Call `POST /v1/orgs` with `{"name": "New Org"}` — expect 201
3. Call `GET /v1/orgs` again

## Expected Result

- Step 2 returns 201 with the new org's `id` and `name`
- Step 3 returns the new org in the `data` array (count increased by 1)
- The new org `id` starts with `org_` and is 36 characters long
