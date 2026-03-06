# Smoke Test Specification

## Abstract

Playwright-based smoke tests verifying core UI and API health against deployed environments (e.g., `dev.everruns.com`). Complements existing localhost-only tests.

## Motivation

Existing tests run against localhost. Smoke tests against deployed environments catch deployment-specific issues: proxy, DNS, TLS, auth provider config, CDN.

## Design

### Architecture

Separate Playwright project (`smoke`) in `apps/ui/playwright.config.ts`. No webServer needed — tests hit a live deployment. Adapts to the target's auth mode (none, admin, full, external) at runtime.

### Environment Variables

| Variable | Source | Description |
|----------|--------|-------------|
| `SMOKE_BASE_URL` | CLI / env | Target URL (default: `https://dev.everruns.com`) |
| `TEST_USER_1_EMAIL` | Doppler | Test user email |
| `TEST_USER_1_NAME` | Doppler | Test user display name |
| `TEST_USER_1_PASSWORD` | Doppler | Test user password (admin/full modes) |
| `PLAYWRIGHT_CHROMIUM_PATH` | env | Custom Chromium path (cloud agents) |

### Proxy Support

Cloud agent environments use an HTTPS egress proxy. The config parses `HTTPS_PROXY` to extract server/username/password and passes them to Playwright's browser context. TLS errors from proxy interception are accepted (`ignoreHTTPSErrors`).

### Test Scope

**Always run (any auth mode):**
1. `GET /health` returns `{"status":"ok"}`
2. `GET /api/v1/auth/config` returns valid config with recognized mode
3. Login page loads (or redirects to dashboard in `none` mode)

**Password-auth modes only (admin/full):**
4. Email/password login redirects to dashboard
5. Dashboard renders heading
6. Sidebar navigation to Agents page
7. Sidebar navigation to Sessions page
8. Logout returns to login page

Tests auto-skip when password auth is disabled (e.g., `external` mode on dev.everruns.com).

### Running

```bash
# Against dev.everruns.com (secrets from Doppler, cloud agent)
PLAYWRIGHT_CHROMIUM_PATH=/root/.cache/ms-playwright/chromium-1194/chrome-linux/chrome \
  doppler run -- npx playwright test --project smoke

# Against a custom environment
SMOKE_BASE_URL=https://staging.everruns.com \
TEST_USER_1_PASSWORD=secret \
  npx playwright test --project smoke
```

### Files

- `apps/ui/playwright.config.ts` — `smoke` project config (proxy, baseURL, ignoreHTTPSErrors)
- `apps/ui/e2e/smoke.spec.ts` — Smoke test suite
- `specs/smoke-test-spec.md` — This spec
