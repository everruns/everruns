import { expect, test, type Page } from "@playwright/test";

// EVE-796 reproduction: the Settings > Organization page "Create Organization"
// button must open the create-organization dialog in a real browser. The
// existing jsdom unit test passes, so this exercises the real base-ui Dialog +
// CSS rendering path that jsdom cannot cover.

const DEFAULT_ORG_ID = "org_00000000000000000000000000000001";

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
        organizations: [
          { public_id: DEFAULT_ORG_ID, name: "Default", role: "owner" },
          { public_id: "org_second", name: "Second Org", role: "member" },
        ],
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
    } else if (/^\/api\/v1\/orgs\/[^/]+$/.test(pathname)) {
      json = {
        id: DEFAULT_ORG_ID,
        name: "Default",
        default_model_id: null,
        default_harness_id: "harness-generic",
        base_harness_id: "harness-base",
      };
    } else if (pathname.endsWith("/harnesses")) {
      json = {
        data: [
          { id: "harness-generic", name: "Generic" },
          { id: "harness-base", name: "Base" },
        ],
        has_more: false,
        total: 2,
      };
    } else if (pathname.endsWith("/config")) {
      json = { policies: {} };
    } else {
      json = { data: [], has_more: false, total: 0 };
    }

    await route.fulfill({ json });
  });
}

test.describe("Settings > Organization create dialog", () => {
  test.beforeEach(async ({ context, page, baseURL }) => {
    const origin = new URL(baseURL ?? "http://localhost:9100");
    await context.addCookies([
      { name: "access_token", value: "e2e-only", domain: origin.hostname, path: "/" },
    ]);
    await mockAppApi(page);
  });

  // Cover both responsive branches of the button: the desktop `xl:w-auto`
  // header button and the narrow-viewport full-width variant.
  for (const viewport of [
    { name: "desktop", width: 1440, height: 2000 },
    { name: "mobile", width: 768, height: 1400 },
  ]) {
    test(`Create Organization button opens the dialog (${viewport.name})`, async ({ page }) => {
      await page.setViewportSize({ width: viewport.width, height: viewport.height });
      await page.goto("/settings/organization");

      const createButton = page.getByRole("button", { name: "Create Organization" });
      await expect(createButton).toBeVisible();
      await createButton.scrollIntoViewIfNeeded();
      await createButton.click();

      // The dialog should render its heading and the name textbox.
      await expect(page.getByRole("heading", { name: "Create Organization" })).toBeVisible();
      const nameInput = page.getByRole("textbox", { name: "Name" });
      await expect(nameInput).toBeVisible();

      // The dialog must be interactive, not merely mounted-but-hidden.
      await nameInput.fill("Repro Org");
      await expect(nameInput).toHaveValue("Repro Org");
    });
  }
});
