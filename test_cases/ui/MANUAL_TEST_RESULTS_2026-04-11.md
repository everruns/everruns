# Manual UI Test Results - 2026-04-11

## Environment

- **Auth Mode**: admin
- **Stack**: API server (in-memory), Caddy proxy, Next.js UI
- **PORT_PREFIX**: 271
- **Browser**: Chromium (headless, via agent-browser)

## Test Summary

| Category | Tests | Pass | Fail/Partial | Issues |
|----------|-------|------|-------------|--------|
| api_keys | 1 | 1 | 0 | 0 |
| **Total** | **1** | **1** | **0** | **0** |

## Detailed Results

### api_keys (1/1 PASS)

- **TC001 API Key Bound to Organisation at Creation**: PASS - API key created while "API Key Binding Test" org is active correctly scopes all downstream operations (agent creation, agent listing) to that org only. Agent created via the API key appeared in the new org and did not appear in the default org, confirming org isolation.

**Verification details:**

1. Created org "API Key Binding Test" via `POST /v1/orgs`
2. Switched to new org via `POST /v1/users/me/switch-org` (server-side cookie set)
3. Created API key `properly-bound-key` via `POST /v1/auth/api-keys` with correct org cookie
4. Created agent `new-org-agent` via `POST /v1/agents` using the API key
5. Verified agent appears in new org (API key auth and session+new-org cookie) but NOT in default org (session+default-org cookie)

**Note:** The `/v1/auth/me` endpoint returns all user memberships regardless of auth method (by design, for UI display). Org binding is enforced by the `ResolvedOrg` middleware on org-scoped endpoints, not by the user info endpoint.
