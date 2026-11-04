import { expect, test, type Page } from "@playwright/test";

const DEFAULT_ORG_ID = "org_00000000000000000000000000000001";
const COLLECTION_ID = "kn_019fda06ccf67d618776620713254657";

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
        evals: false,
        knowledge: true,
        voice: false,
        observers: false,
        public_chat: false,
      };
    } else if (pathname.endsWith("/switch-org")) {
      json = { success: true, org_id: DEFAULT_ORG_ID };
    } else if (pathname === "/api/v1/virtual-users/me/connections") {
      json = [];
    } else if (pathname === "/api/v1/models") {
      json = {
        data: [
          {
            id: "model-embedding",
            model_id: "text-embedding-3-small",
            display_name: "text-embedding-3-small",
            provider_id: "provider-openai",
            provider_name: "OpenAI",
            provider_type: "openai",
            healthy: true,
            enabled: true,
            capabilities: ["embeddings"],
          },
        ],
        has_more: false,
        total: 1,
      };
    } else if (/^\/api\/v1\/orgs\/[^/]+$/.test(pathname)) {
      json = {
        id: DEFAULT_ORG_ID,
        name: "Default",
        default_model_id: null,
        default_harness_id: null,
        base_harness_id: null,
      };
    } else {
      json = { data: [], has_more: false, total: 0 };
    }

    if (pathname === "/api/v1/knowledge") {
      json = {
        data: [
          {
            id: COLLECTION_ID,
            name: "Bashkit Knowledge",
            description:
              "A deliberately long knowledge collection description that must remain readable on narrow screens.",
            indexing: "off",
            status: "active",
          },
        ],
        next_cursor: null,
      };
    }

    await route.fulfill({ json });
  });
}

test.describe("Knowledge responsive collections", () => {
  test.beforeEach(async ({ context, page, baseURL }) => {
    const origin = new URL(baseURL ?? "http://localhost:9100");
    await context.addCookies([
      { name: "access_token", value: "e2e-only", domain: origin.hostname, path: "/" },
    ]);
    await mockAppApi(page);
  });

  for (const viewport of [
    { name: "320 mobile", width: 320, height: 900 },
    { name: "375 mobile", width: 375, height: 900 },
    { name: "390 mobile", width: 390, height: 900 },
    { name: "tablet", width: 768, height: 1000 },
    { name: "compact desktop", width: 1024, height: 1000 },
    { name: "desktop", width: 1440, height: 1000 },
  ]) {
    test(`contains collections and actions at ${viewport.name}`, async ({ page }) => {
      await page.setViewportSize({ width: viewport.width, height: viewport.height });
      await page.goto("/knowledge");
      await expect(page.getByRole("heading", { name: "Knowledge", exact: true })).toBeVisible();
      const card = page.getByRole("link", { name: /Bashkit Knowledge/ });
      await expect(card).toHaveAttribute("href", `/knowledge/${COLLECTION_ID}`);
      await expect(page.getByText("Files only", { exact: true })).toBeVisible();
      await expect(page.getByText(/A deliberately long knowledge collection/)).toBeVisible();
      await card.click({ trial: true });
      const box = await card.boundingBox();
      expect(box).not.toBeNull();
      expect(box!.x).toBeGreaterThanOrEqual(0);
      expect(box!.x + box!.width).toBeLessThanOrEqual(viewport.width);
      const create = page.getByRole("button", { name: "New collection", exact: true });
      await expect(create).toBeVisible();
      await create.click({ trial: true });
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        await page.evaluate(() => document.documentElement.clientWidth),
      );
    });
  }

  test("redirects the retired indexes page to Knowledge", async ({ page }) => {
    await page.goto("/knowledge-indexes");
    await expect(page).toHaveURL(/\/knowledge$/);
    await expect(page.getByRole("link", { name: /Bashkit Knowledge/ })).toBeVisible();
  });
});

test.describe("Developer showcase containment", () => {
  for (const route of ["/dev/markdown", "/dev/tool-activity"]) {
    test(`${route} contains production previews at 320px`, async ({ page }) => {
      await page.setViewportSize({ width: 320, height: 900 });
      await page.goto(route);

      await expect(page.locator("h1")).toBeVisible();
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        await page.evaluate(() => document.documentElement.clientWidth),
      );
    });
  }
});

test.describe("Auth page containment", () => {
  test.beforeEach(async ({ page }) => {
    await page.route("**/v1/auth/config", (route) =>
      route.fulfill({
        json: {
          mode: "full",
          password_auth_enabled: true,
          oauth_providers: ["google", "github"],
          signup_enabled: true,
          signup_email_confirm: true,
        },
      }),
    );
  });

  for (const viewport of [
    { name: "320 mobile", width: 320, height: 900 },
    { name: "375 mobile", width: 375, height: 900 },
    { name: "390 mobile", width: 390, height: 900 },
    { name: "tablet", width: 768, height: 1000 },
    { name: "compact desktop", width: 1024, height: 1000 },
    { name: "desktop", width: 1440, height: 1000 },
  ]) {
    test(`contains auth and recovery forms at ${viewport.name}`, async ({ page }) => {
      await page.setViewportSize(viewport);

      for (const route of ["/login", "/signup", "/forgot-password", "/verify-email"]) {
        await page.goto(route);
        await expect(page.locator("h1")).toBeVisible();
        expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
          await page.evaluate(() => document.documentElement.clientWidth),
        );
      }
    });
  }
});
