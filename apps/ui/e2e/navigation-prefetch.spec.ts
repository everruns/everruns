import { expect, test, type Page, type Request } from "@playwright/test";

const DEFAULT_ORG_ID = "org_00000000000000000000000000000001";
// next dev compiles a destination's RSC bundle on its first navigation.
const NAVIGATION_TIMEOUT = 30_000;
const SETTINGS_CHILD_ROUTES = [
  "/settings/organization",
  "/settings/tracing",
  "/settings/providers",
  "/settings/members",
  "/settings/features",
  "/settings/payments",
  "/settings/profile",
  "/settings/user-profile",
  "/settings/connections",
  "/settings/mcp-servers",
  "/settings/ai-clients",
  "/settings/personal-access-tokens",
];

type Layout = "chat_first" | "classic";

async function mockAppApi(page: Page, layout: Layout) {
  await page.route("**/api/v1/**", async (route) => {
    const pathname = new URL(route.request().url()).pathname;
    let json: unknown;

    if (pathname === "/api/v1/platform/auth/config") {
      json = {
        mode: "none",
        password_auth_enabled: false,
        oauth_providers: [],
        signup_enabled: false,
      };
    } else if (pathname === "/api/v1/platform/auth/me") {
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
        evals: false,
        voice: false,
        observers: false,
        public_chat: false,
      };
    } else if (pathname.endsWith("/switch-org")) {
      json = { success: true, org_id: DEFAULT_ORG_ID };
    } else if (pathname === "/api/v1/platform/user/preferences/ui.layout") {
      json = { key: "ui.layout", value: layout, updated_at: "2026-11-11T00:00:00Z" };
    } else if (pathname.endsWith("/config")) {
      json = { policies: {} };
    } else if (pathname.endsWith("/quality-monitoring")) {
      json = {
        policy: {
          enabled: false,
          sampling_rate: 0.02,
          model_id: null,
          daily_budget_usd: 0,
          excluded_tags: [],
        },
        observer_id: null,
        today: { reserved_usd: 0, reported_usd: 0 },
        checks: [],
      };
    } else if (pathname.endsWith("/quality-monitoring/scores")) {
      json = [];
    } else if (pathname === "/api/v1/platform/sessions/facets") {
      json = {
        active_now: 0,
        by_activity: [],
        by_agent: [],
        by_source: [],
        failed_today: 0,
        p95_duration_ms: 0,
        tokens_today: 0,
        total: 0,
      };
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
  "/api/v1/platform/auth/config",
  "/api/v1/platform/auth/me",
  "/api/v1/platform/agents",
  // Chats needs the harness list to start a thread (EVE-855: Chats is the
  // landing surface, so this is now a startup call rather than a navigation).
  "/api/v1/platform/harnesses",
  "/api/v1/platform/capabilities",
  "/api/v1/platform/models",
  "/api/v1/platform/providers",
  // Chats reads the provider policy map to decide whether the "no intelligence
  // available" notice may link to Settings. Only fetched when the org actually
  // has no usable chat model — which is this suite's mocked state, since the
  // API mock answers every list with `{ data: [] }`.
  "/api/v1/platform/providers/config",
  "/api/v1/platform/sessions",
  "/api/v1/platform/sessions/platform-chat",
  "/api/v1/platform/agents/platform-chat",
  "/api/v1/platform/sessions/stats",
  "/api/v1/platform/feature-flags",
  "/api/v1/platform/users/me/switch-org",
  "/api/v1/platform/user/preferences/ui.early_access_banner.dismissed",
  // The personal layout preference (Chat first or Classic) shapes the shell.
  "/api/v1/platform/user/preferences/ui.layout",
  // How Chat opens in Classic (full screen, floating or docked) shapes it too.
  "/api/v1/platform/user/preferences/ui.classic_chat",
  "/api/v1/platform/durable/config",
  // The sidebar's Overview entry counts the items waiting on Overview, so it
  // reads the same sources as that page on every route.
  "/api/v1/platform/agents/activity",
  "/api/v1/platform/agent-examples",
  "/api/v1/platform/health-issues",
  "/api/v1/platform/sandboxes/stats",
  "/api/v1/platform/mcp-servers/config",
  // Settings > Plugins is shown to people who hold `plugin.view`.
  "/api/v1/platform/plugins/config",
  // Shared navigation gates the Extractions destination on model management.
  // The small policy map is required by the shell; extraction data is never prefetched.
  "/api/v1/platform/models/config",
]);

function isStartupApi(pathname: string) {
  return (
    STARTUP_API_ALLOWLIST.has(pathname) ||
    /^\/api\/v1\/platform\/orgs\/[^/]+(?:\/feature-flags)?$/.test(pathname)
  );
}

for (const layout of ["chat_first", "classic"] as const)
  test.describe(`Navigation prefetch (${layout})`, () => {
    test.setTimeout(60_000);
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
      await mockAppApi(page, layout);
    });

    test("Landing-surface startup does not prefetch unrelated sidebar routes or APIs", async ({
      page,
    }) => {
      const requests: Request[] = [];
      page.on("request", (request) => requests.push(request));

      await page.goto("/chats");
      await expect(page.getByRole("link", { name: "Sessions" })).toBeVisible();
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

    for (const { adopted, canManageModels } of [
      { adopted: true, canManageModels: true },
      { adopted: true, canManageModels: false },
      { adopted: false, canManageModels: true },
    ]) {
      test(`Extractions navigation with adoption=${adopted} and model management=${canManageModels}`, async ({
        page,
      }) => {
        await page.route("**/feature-flags", (route) =>
          route.fulfill({ json: { extractions: adopted } }),
        );
        await page.route("**/api/v1/platform/models/config", (route) =>
          route.fulfill({ json: { policies: { "model.manage": canManageModels } } }),
        );
        await page.goto("/chats");
        await expect(page.getByRole("link", { name: "Sessions" })).toBeVisible();
        await page.waitForLoadState("networkidle");
        const extractionLink = page.getByRole("link", { name: "Extractions", exact: true });
        if (adopted && canManageModels) {
          await expect(extractionLink).toBeVisible();
          await extractionLink.click();
          await expect(page).toHaveURL(/\/extractions$/, { timeout: NAVIGATION_TIMEOUT });
          await expect(page.getByLabel("Source text")).toBeVisible();
        } else {
          await expect(extractionLink).toHaveCount(0);
        }
        if (!adopted) {
          await page.goto("/extractions");
          await expect(page.getByText("Extractions is not enabled")).toBeVisible();
        }
      });
    }

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
      await expect(page).toHaveURL(/\/sessions$/, { timeout: NAVIGATION_TIMEOUT });
      await expect(
        page.getByText("Every run across your agents: chats, schedules, channels and API calls."),
      ).toBeVisible();
    });

    test("Settings navigation avoids child route fan-out", async ({ page }) => {
      const settingsRequests: Request[] = [];
      page.on("request", (request) => {
        if (isSettingsRscRequest(request)) settingsRequests.push(request);
      });

      await page.goto("/chats");
      await expect(page.getByRole("link", { name: "Sessions" })).toBeVisible();
      await page.waitForLoadState("networkidle");

      settingsRequests.length = 0;
      if (layout === "chat_first") {
        // The rail folds Settings into its System and settings menu.
        await page.getByRole("button", { name: "System and settings" }).click();
        await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
      } else {
        await page.getByRole("link", { name: "Settings", exact: true }).click();
      }
      await expect(page).toHaveURL(/\/settings\/organization$/, { timeout: NAVIGATION_TIMEOUT });
      await page.waitForLoadState("networkidle");

      const eagerChildRequests = settingsRequests.filter((request) => {
        const pathname = new URL(request.url()).pathname;
        return pathname !== "/settings/organization" && SETTINGS_CHILD_ROUTES.includes(pathname);
      });
      expect(eagerChildRequests).toHaveLength(0);
    });
  });
