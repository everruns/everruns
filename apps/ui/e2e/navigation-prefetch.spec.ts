import { expect, test, type Page, type Request } from "@playwright/test";

const DEFAULT_ORG_ID = "org_00000000000000000000000000000001";
const SETTINGS_CHILD_ROUTES = [
  "/settings/organization",
  "/settings/providers",
  "/settings/members",
  "/settings/features",
  "/settings/payments",
  "/settings/profile",
  "/settings/connections",
  "/settings/personal-access-tokens",
];

async function mockAppApi(page: Page) {
  await page.route("**/api/v1/**", async (route) => {
    const pathname = new URL(route.request().url()).pathname;
    let json: unknown;

    if (pathname === "/api/v1/auth/config") {
      json = {
        mode: "none",
        password_auth_enabled: false,
        oauth_providers: [],
        signup_enabled: false,
      };
    } else if (pathname === "/api/v1/auth/me") {
      json = {
        id: "user-1",
        email: "dev@example.com",
        name: "Dev User",
        roles: [],
        email_verified: true,
        organizations: [{ public_id: DEFAULT_ORG_ID, name: "Default", role: "owner" }],
      };
    } else if (pathname.endsWith("/feature-flags")) {
      json = {
        notifications: false,
        evals: false,
        app_budgets: false,
        agent_versions: false,
        voice: false,
        agent_delegation: false,
        observers: false,
        public_chat: false,
      };
    } else if (pathname.endsWith("/switch-org")) {
      json = { success: true, org_id: DEFAULT_ORG_ID };
    } else if (pathname.endsWith("/config")) {
      json = { policies: {} };
    } else {
      json = { data: [], has_more: false, total: 0 };
    }

    await route.fulfill({ json });
  });
}

function isSettingsRscRequest(request: Request) {
  const url = new URL(request.url());
  return request.headers().rsc === "1" && url.pathname.startsWith("/settings");
}

function isRscRequest(request: Request) {
  return request.headers().rsc === "1";
}

// APIs the landing surface legitimately calls on startup. The point of the
// tests below is that nothing *else* is fetched — not that this set is minimal.
const STARTUP_API_ALLOWLIST = new Set([
  "/api/v1/auth/config",
  "/api/v1/auth/me",
  "/api/v1/agents",
  // Chats needs the harness list to start a thread (EVE-855: Chats is the
  // landing surface, so this is now a startup call rather than a navigation).
  "/api/v1/harnesses",
  "/api/v1/capabilities",
  "/api/v1/models",
  "/api/v1/providers",
  "/api/v1/sessions",
  "/api/v1/sessions/stats",
  "/api/v1/feature-flags",
  "/api/v1/users/me/switch-org",
  "/api/v1/user/preferences/ui.early_access_banner.dismissed",
  "/api/v1/durable/config",
]);

function isStartupApi(pathname: string) {
  return (
    STARTUP_API_ALLOWLIST.has(pathname) ||
    /^\/api\/v1\/orgs\/[^/]+(?:\/feature-flags)?$/.test(pathname)
  );
}

test.describe("Sidebar navigation prefetch", () => {
  test.use({ viewport: { width: 1440, height: 2000 } });

  test.beforeEach(async ({ context, page, baseURL }) => {
    const origin = new URL(baseURL ?? "http://localhost:9100");
    await context.addCookies([
      {
        name: "access_token",
        value: "e2e-only",
        domain: origin.hostname,
        path: "/",
      },
    ]);
    await mockAppApi(page);
  });

  test("Landing-surface startup does not prefetch unrelated sidebar routes or APIs", async ({ page }) => {
    const requests: Request[] = [];
    page.on("request", (request) => requests.push(request));

    await page.goto("/chats");
    await expect(page.getByRole("link", { name: "Settings", exact: true })).toBeVisible();
    await page.waitForLoadState("networkidle");

    const unrelatedRscRequests = requests.filter((request) => {
      const pathname = new URL(request.url()).pathname;
      return isRscRequest(request) && pathname !== "/chats";
    });
    expect(unrelatedRscRequests).toHaveLength(0);

    const unrelatedApiRequests = requests.filter((request) => {
      const pathname = new URL(request.url()).pathname;
      return pathname.startsWith("/api/v1/") && !isStartupApi(pathname);
    });
    expect(unrelatedApiRequests.map((request) => new URL(request.url()).pathname)).toEqual([]);
  });

  // EVE-785's dashboard-specific cases are gone with the Dashboard (EVE-855).
  // They asserted that the empty Dashboard's three "Create your first agent"
  // links did not eagerly prefetch /agents/new. Chats, the landing surface now,
  // offers a new-chat form instead and has no such link; the surviving surface
  // is /agents, covered below.
  //
  // EVE-793: the Agents-page /agents/new prefetch regression is covered
  // deterministically by the jest test
  // `src/__tests__/agents-page-new-agent-prefetch.test.tsx` (both the masthead
  // and empty-state links assert prefetch is disabled and fires only on
  // hover/focus intent). It is intentionally not duplicated here: /agents
  // renders its header from a server-side fetch that this suite's browser-level
  // `page.route` mocks cannot intercept, so the page never reaches a stable
  // rendered state under these mocks (unlike the client-fetched landing surface
  // widget), which made an /agents e2e case non-deterministic.

  test("sidebar click navigation works without broad route prefetch", async ({ page }) => {
    await page.goto("/chats");
    await expect(page.getByRole("link", { name: "Sessions" })).toBeVisible();
    await page.getByRole("link", { name: "Sessions" }).click();
    await expect(page).toHaveURL(/\/sessions$/);
  });

  test("Settings navigation avoids child route fan-out", async ({ page }) => {
    const settingsRequests: Request[] = [];
    page.on("request", (request) => {
      if (isSettingsRscRequest(request)) settingsRequests.push(request);
    });

    await page.goto("/chats");
    await expect(page.getByRole("link", { name: "Settings", exact: true })).toBeVisible();
    await page.waitForLoadState("networkidle");

    settingsRequests.length = 0;
    await page.getByRole("link", { name: "Settings", exact: true }).click();
    await expect(page).toHaveURL(/\/settings\/organization$/);
    await page.waitForLoadState("networkidle");

    const eagerChildRequests = settingsRequests.filter((request) => {
      const pathname = new URL(request.url()).pathname;
      return pathname !== "/settings/organization" && SETTINGS_CHILD_ROUTES.includes(pathname);
    });
    expect(eagerChildRequests).toHaveLength(0);
  });
});
