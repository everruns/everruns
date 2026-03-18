# TC003: Get Created Organization by ID

## Description

Verifies that a newly created organization can be fetched by its ID immediately after creation.

## Preconditions

- User is authenticated

## Steps

1. Call `POST /v1/orgs` with `{"name": "Fetch Me"}` — expect 201
2. Note the returned `id` field
3. Call `GET /v1/orgs/<id from step 2>`

## Expected Result

- Step 3 returns 200 with the org details including `name: "Fetch Me"`
