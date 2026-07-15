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
        global_chat: false,
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

test.describe("Settings navigation prefetch", () => {
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

  test("normal pages and Settings navigation avoid Settings route fan-out", async ({ page }) => {
    const settingsRequests: Request[] = [];
    page.on("request", (request) => {
      if (isSettingsRscRequest(request)) settingsRequests.push(request);
    });

    await page.goto("/dashboard");
    await expect(page.getByRole("link", { name: "Settings" })).toBeVisible();
    await page.waitForLoadState("networkidle");

    expect(settingsRequests).toHaveLength(0);

    settingsRequests.length = 0;
    await page.getByRole("link", { name: "Settings" }).click();
    await expect(page).toHaveURL(/\/settings\/organization$/);
    await page.waitForLoadState("networkidle");

    const eagerChildRequests = settingsRequests.filter((request) => {
      const pathname = new URL(request.url()).pathname;
      return pathname !== "/settings/organization" && SETTINGS_CHILD_ROUTES.includes(pathname);
    });
    expect(eagerChildRequests).toHaveLength(0);
  });
});
