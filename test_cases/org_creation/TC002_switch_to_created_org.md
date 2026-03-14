# TC002: Switch to Newly Created Organization

## Description

Verifies that a user can switch to an organization they just created, without re-authenticating.

## Preconditions

- User is authenticated

## Steps

1. Call `POST /v1/orgs` with `{"name": "Switch Target"}` — expect 201
2. Note the returned `id` field
3. Call `POST /v1/users/me/switch-org` with `{"org_id": "<id from step 2>"}`

## Expected Result

- Step 3 returns 200 with `{"success": true, "org_id": "<id>"}`
- An `everruns_org` cookie is set with the new org ID
