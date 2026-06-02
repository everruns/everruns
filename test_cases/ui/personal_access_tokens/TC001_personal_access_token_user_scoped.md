# TC001: Personal Access Token User-Scoped Access

Supersedes the EVE-274 regression test. Personal access tokens are user-scoped (tied to a person, not org-scoped).

## Description

Verify that a personal access token created through the UI grants access to all organizations the user belongs to. The token is user-scoped — organization context is resolved per-request via `X-Org-Id` header or `everruns_org` cookie.

## Preconditions

- Server running with authentication enabled (`just start-all`)
- User belongs to at least two organizations

## Test Data

| Field      | Value               |
| ---------- | ------------------- |
| Token Name | `user-scoped-token` |

## Steps

1. Navigate to the login page and sign in
2. Navigate to Settings > Personal access tokens
3. Verify the "Full account access" warning banner is visible and states tokens are tied to your user account (not an organization)
4. Click "Create token"
5. Enter name `user-scoped-token`, keep the default `90 days` expiration
6. Click "Create token"
7. Copy the full token (starts with `evr_pat_`) from the "Personal access token created" dialog
8. Click "Done"
9. Verify the token `user-scoped-token` appears in the list

**API verification** (curl):

10. Create an agent in org A: `POST /v1/agents` with `Authorization: Bearer <token>` and `X-Org-Id: <org-a-public-id>`
11. Create an agent in org B: `POST /v1/agents` with `Authorization: Bearer <token>` and `X-Org-Id: <org-b-public-id>`
12. List agents in org A: `GET /v1/agents` with `Authorization: Bearer <token>` and `X-Org-Id: <org-a-public-id>`
13. List agents in org B: `GET /v1/agents` with `Authorization: Bearer <token>` and `X-Org-Id: <org-b-public-id>`
14. Call without `X-Org-Id` when user has multiple orgs: `GET /v1/agents` with `Authorization: Bearer <token>` only

## Expected Result

- Step 3: Warning banner says tokens are tied to your user account and grant access to all organizations
- Step 5: `90 days` is selected by default in the expiration presets
- Steps 10-11: Both agents created successfully (same token, different orgs)
- Step 12: Only org A's agent appears
- Step 13: Only org B's agent appears
- Step 14: Returns 400 "Multiple organizations available. Specify the target organization via the X-Org-Id header."
