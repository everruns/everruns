# TC001: API Key Bound to Organisation at Creation

Regression test for [EVE-274](https://linear.app/everruns/issue/EVE-274).

## Description

Verify that an API key created **through the UI** after creating and switching to a new organisation is bound to that new org — not the default org. This catches stale JWT issues where the `ResolvedOrg` extractor validates the org cookie against JWT-derived memberships instead of the database, causing silent fallback to the default org.

## Preconditions

- Server running with authentication enabled (`just start-all`)
- **No existing session** — start from a fresh login (clear cookies or use incognito)

## Test Data

| Field            | Value                  |
| ---------------- | ---------------------- |
| Org Name         | `API Key Binding Test` |
| API Key Name     | `org-bound-key`        |

## Steps

**Important:** Steps 1–13 must all be performed in the browser (agent-browser). Do NOT use curl or API calls for org creation or switching — that bypasses the stale-JWT code path this test targets.

1. Navigate to the login page and sign in
2. Navigate to Settings > Organisation
3. Click "Create Organisation"
4. Enter org name `API Key Binding Test` and click "Create"
5. Wait for setup page to complete (all three steps show green checkmarks)
6. Click "Go to dashboard" (or navigate to dashboard)
7. **Verify sidebar org dropdown shows `API Key Binding Test`** as active org (not "Default Organization")
8. Navigate to Settings > API Keys
9. Click "Create API Key"
10. Enter name `org-bound-key`, leave expiration empty
11. Click "Create API Key"
12. Copy the full API key from the "API Key Created" dialog
13. Click "Done"
14. Verify the key `org-bound-key` appears in the API keys list with its prefix

**API verification** (curl is fine for these read-only checks):

15. Create an agent via `POST /v1/agents` with header `Authorization: Bearer <key>`
16. List agents via the API key: `GET /v1/agents` with `Authorization: Bearer <key>`
17. List agents via session auth with the **default** org cookie (`everruns_org=org_000...001`)
18. List agents via session auth with the **new** org cookie

## Expected Result

- Steps 1–14: API key created successfully through the full UI flow
- Step 7: Sidebar **must** show `API Key Binding Test`, not `Default Organization` — if it shows the default org, the org switch failed (likely EVE-274)
- Step 16: Agent appears when listing via the API key
- Step 17: Agent does **not** appear in the default org
- Step 18: Agent **does** appear in the new org

## Failure Signature (EVE-274)

If the bug is present, the following happens silently:

1. Steps 1–14 appear to succeed (no UI error)
2. But step 17 shows the agent in the **default** org
3. And step 18 shows **zero** agents in the new org
4. Root cause: the `ResolvedOrg` extractor validated the cookie against a stale JWT (which didn't include the new org), fell back to the default org, and the API key was bound there
