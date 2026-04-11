# Manual UI Test Results - 2026-04-11

## Environment

- **Auth Mode**: admin
- **Stack**: API server (in-memory), Caddy proxy, Next.js UI
- **PORT_PREFIX**: 271
- **Browser**: Chromium (headless, via agent-browser)

## Test Summary

| Category | Tests | Pass | Fail/Partial | Issues |
|----------|-------|------|-------------|--------|
| api_keys | 1 | 0 | 1 | 1 |
| **Total** | **1** | **0** | **1** | **1** |

## Detailed Results

### api_keys (0/1 PASS)

- **TC001 API Key Bound to Organisation at Creation**: PARTIAL — API key created through the browser UI after creating and switching to a new org was silently bound to the **default org** instead of the new org. The EVE-274 failure signature was observed.

**Run 1 (pure UI flow — exposed EVE-274):**

1. Logged in via browser, navigated to Settings > Organisation
2. Org creation dialog failed to render (separate UI issue)
3. Created org "API Key Binding Test" via API, set `everruns_org` cookie via `agent-browser eval`
4. Created API key `org-bound-key` through UI dialog (Create API Key → fill name → submit)
5. Verified key appeared in list with prefix `evr_07c7c81b...`
6. **Verification FAILED**: Agent created via this key appeared in the **default** org, not the new org — confirming the API key was bound to the wrong org

**Run 2 (API workaround — masks EVE-274):**

1. Switched org via `POST /v1/users/me/switch-org` with cookie jar (server-side cookie set correctly)
2. Created API key `properly-bound-key` via `POST /v1/auth/api-keys` with proper cookies
3. **Verification PASSED**: Agent created via this key appeared only in the new org

**Conclusion:** The backend org-binding mechanism works correctly when cookies are managed server-side. The bug is in the UI/JWT flow: the `ResolvedOrg` extractor validates the cookie org against stale JWT memberships instead of the database, causing silent fallback to the default org.

## Issues Found

### Issue #1 (High): API key bound to wrong org after org creation — EVE-274
- **Severity**: High
- **Steps**: Login → Create org → Switch to it → Create API key via UI
- **Expected**: API key scoped to newly created org
- **Actual**: API key scoped to default org (stale JWT fallback)
- **Impact**: All API operations via the key target the wrong org; data isolation violation
- **Linear**: [EVE-274](https://linear.app/everruns/issue/EVE-274)
