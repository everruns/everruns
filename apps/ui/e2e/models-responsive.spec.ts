import { expect, test, type Page } from "@playwright/test";

const DEFAULT_ORG_ID = "org_00000000000000000000000000000001";

const providers = [
  {
    id: "provider-openai",
    name: "OpenAI",
    provider_type: "openai",
    status: "active",
    api_key_set: true,
    base_url: null,
    created_at: "2026-07-01T00:00:00Z",
    updated_at: "2026-07-01T00:00:00Z",
  },
];

const models = [
  {
    id: "model-gpt-5-6-luna",
    model_id: "gpt-5.6-luna",
    display_name: "GPT-5.6 Luna",
    provider_id: "provider-openai",
    provider_name: "OpenAI",
    provider_type: "openai",
    model_vendor: "openai",
    healthy: true,
    enabled: true,
    capabilities: ["chat", "function_calling", "vision"],
    profile: {
      name: "GPT-5.6 Luna",
      family: "gpt-5.6-luna",
      release_date: "2026-07-09",
      reasoning: true,
      tool_call: true,
      attachment: true,
      structured_output: true,
    },
    created_at: "2026-07-09T00:00:00Z",
    updated_at: "2026-07-09T00:00:00Z",
  },
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
    } else if (pathname === "/api/v1/providers") {
      json = { data: providers, has_more: false, total: providers.length };
    } else if (pathname === "/api/v1/models") {
      json = { data: models, has_more: false, total: models.length };
    } else if (/^\/api\/v1\/orgs\/[^/]+$/.test(pathname)) {
      json = {
        id: DEFAULT_ORG_ID,
        name: "Default",
        default_model_id: models[0].id,
        default_harness_id: null,
        base_harness_id: null,
      };
    } else {
      json = { data: [], has_more: false, total: 0 };
    }

    await route.fulfill({ json });
  });
}

test.describe("Models responsive layout", () => {
  test.beforeEach(async ({ context, page, baseURL }) => {
    const origin = new URL(baseURL ?? "http://localhost:9100");
    await context.addCookies([
      { name: "access_token", value: "e2e-only", domain: origin.hostname, path: "/" },
    ]);
    await mockAppApi(page);
  });

  for (const viewport of [
    { name: "compact desktop", width: 1024, height: 900 },
    { name: "tablet", width: 768, height: 900 },
  ]) {
    test(`keeps model content inside its row at ${viewport.name} width`, async ({ page }) => {
      await page.setViewportSize({ width: viewport.width, height: viewport.height });
      await page.goto("/models");

      const disableButton = page.getByRole("button", { name: "Disable" });
      await expect(disableButton).toBeVisible();

      const row = disableButton.locator(
        "xpath=ancestor::div[contains(@class, 'overflow-hidden')][1]",
      );
      const providerRail = page
        .getByRole("link", { name: "OpenAI 1" })
        .locator("xpath=ancestor::div[contains(@class, 'bg-card')][1]");
      const rowBox = await row.boundingBox();
      const buttonBox = await disableButton.boundingBox();
      const providerRailBox = await providerRail.boundingBox();

      expect(rowBox).not.toBeNull();
      expect(buttonBox).not.toBeNull();
      expect(providerRailBox).not.toBeNull();
      expect(buttonBox!.x + buttonBox!.width).toBeLessThanOrEqual(rowBox!.x + rowBox!.width);
      expect(providerRailBox!.x).toBe(rowBox!.x);
      expect(providerRailBox!.y).toBeGreaterThan(rowBox!.y + rowBox!.height);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        await page.evaluate(() => document.documentElement.clientWidth),
      );
    });
  }
});
