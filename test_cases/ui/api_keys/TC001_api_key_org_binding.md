# TC001: API Key Bound to Organisation at Creation

## Description

Verify that an API key created while a specific organisation is active is permanently bound to that organisation. When the API key is used for authentication, all org-scoped operations are restricted to the org that was active at creation time.

## Preconditions

- Server running with authentication enabled (`just start-all` or `doppler run -- just start-dev` with `AUTH_MODE=admin`)
- User is authenticated via browser session
- User has permission to create organisations

## Test Data

| Field            | Value                  |
| ---------------- | ---------------------- |
| Org Name         | `API Key Binding Test` |
| API Key Name     | `org-bound-key`        |

## Steps

1. Navigate to Settings > Organisation
2. Click "Create Organisation"
3. Enter org name `API Key Binding Test` and click "Create"
4. Wait for setup page to complete (all three steps show green checkmarks)
5. Click "Go to dashboard" (or navigate to dashboard)
6. Confirm the sidebar org dropdown shows `API Key Binding Test` as active org
7. Navigate to Settings > API Keys
8. Click "Create API Key"
9. Enter name `org-bound-key`, leave expiration empty
10. Click "Create API Key"
11. Copy the full API key from the "API Key Created" dialog
12. Click "Done"
13. Verify the key `org-bound-key` appears in the API keys list with its prefix
14. Using the copied API key, create an agent via `POST /v1/agents` with header `Authorization: Bearer <key>`
15. List agents via the API key: `GET /v1/agents` with `Authorization: Bearer <key>`
16. List agents via session auth with the original (default) org cookie
17. List agents via session auth with the new org cookie

## Expected Result

- API key is created successfully and shown once in the dialog
- Key appears in the API keys list with name `org-bound-key`
- Agent created via the API key appears **only** in the `API Key Binding Test` org (step 15 and step 17)
- Agent does **not** appear in the default org (step 16)
- This proves the API key's org binding restricts all downstream operations to the bound org
