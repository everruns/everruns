# TC001: API Key User-Scoped Access

Supersedes the EVE-274 regression test. API keys are now user-scoped (not org-scoped).

## Description

Verify that an API key created through the UI grants access to all organizations the user belongs to. The key is user-scoped — organization context is resolved per-request via `X-Org-Id` header or `everruns_org` cookie.

## Preconditions

- Server running with authentication enabled (`just start-all`)
- User belongs to at least two organizations

## Test Data

| Field            | Value                  |
| ---------------- | ---------------------- |
| API Key Name     | `user-scoped-key`      |

## Steps

1. Navigate to the login page and sign in
2. Navigate to Settings > API Keys
3. Verify the "Full account access" warning banner is visible
4. Click "Create API Key"
5. Enter name `user-scoped-key`, leave expiration empty
6. Click "Create API Key"
7. Copy the full API key from the "API Key Created" dialog
8. Click "Done"
9. Verify the key `user-scoped-key` appears in the API keys list

**API verification** (curl):

10. Create an agent in org A: `POST /v1/agents` with `Authorization: Bearer <api-key>` and `X-Org-Id: <org-a-public-id>`
11. Create an agent in org B: `POST /v1/agents` with `Authorization: Bearer <api-key>` and `X-Org-Id: <org-b-public-id>`
12. List agents in org A: `GET /v1/agents` with `Authorization: Bearer <api-key>` and `X-Org-Id: <org-a-public-id>`
13. List agents in org B: `GET /v1/agents` with `Authorization: Bearer <api-key>` and `X-Org-Id: <org-b-public-id>`
14. Call without `X-Org-Id` when user has multiple orgs: `GET /v1/agents` with `Authorization: Bearer <api-key>` only

## Expected Result

- Step 3: Warning banner says API keys grant access to all organizations
- Steps 10-11: Both agents created successfully (same key, different orgs)
- Step 12: Only org A's agent appears
- Step 13: Only org B's agent appears
- Step 14: Returns 400 "Multiple organizations available. Specify the target organization via the X-Org-Id header."
