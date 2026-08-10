import { expect, test, type Page } from "@playwright/test";

const DEFAULT_ORG_ID = "org_00000000000000000000000000000001";
const INDEX_ID = "kidx_019fda06ccf67d618776620713254657";

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
        knowledge: true,
        app_budgets: false,
        agent_versions: false,
        voice: false,
        agent_delegation: false,
        observers: false,
        public_chat: false,
      };
    } else if (pathname.endsWith("/switch-org")) {
      json = { success: true, org_id: DEFAULT_ORG_ID };
    } else if (pathname === "/api/v1/user/connections") {
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

    if (pathname === "/api/v1/knowledge-indexes") {
      json = {
        data: [
          {
            id: INDEX_ID,
            name: "Bashkit Knowledge",
            description:
              "A deliberately long knowledge index description that must remain readable on narrow screens.",
            source_type: "github",
            source_config: {
              provider: "github",
              repository: "everruns/bashkit",
              branch: "main",
            },
            embedding_model_id: "model-embedding",
            vector_dim: 1536,
            document_count: 0,
            status: "active",
            sync_status: "pending",
            last_synced_at: null,
            last_sync_error: null,
            created_at: "2026-08-01T00:00:00Z",
            updated_at: "2026-08-01T00:00:00Z",
            archived_at: null,
            deleted_at: null,
          },
        ],
        has_more: false,
        total: 1,
      };
    }

    await route.fulfill({ json });
  });
}

test.describe("Knowledge indexes responsive records", () => {
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
  ]) {
    test(`stacks fields and contains every action at ${viewport.name}`, async ({ page }) => {
      await page.setViewportSize({ width: viewport.width, height: viewport.height });
      await page.goto("/knowledge-indexes");

      const table = page.getByRole("table", { name: "Knowledge indexes" });
      const row = table.locator('[data-slot="responsive-table-row"]');
      await expect(row).toBeVisible();
      await expect(table.getByRole("columnheader", { name: "Name" })).toBeHidden();
      await expect(row.getByText("Source", { exact: true })).toBeVisible();
      await expect(row.getByText("Index state", { exact: true })).toBeVisible();
      await expect(row.getByText(INDEX_ID, { exact: true })).toHaveCount(0);
      await expect(row.getByRole("button", { name: `Copy ID: ${INDEX_ID}` })).toBeVisible();

      for (const action of ["Open", "Edit", "Archive"]) {
        const control = row.getByRole(action === "Open" ? "link" : "button", {
          name: action,
          exact: true,
        });
        await expect(control).toBeVisible();
        await control.click({ trial: true });
        const controlBox = await control.boundingBox();
        const rowBox = await row.boundingBox();
        expect(controlBox).not.toBeNull();
        expect(rowBox).not.toBeNull();
        expect(controlBox!.x).toBeGreaterThanOrEqual(rowBox!.x);
        expect(controlBox!.x + controlBox!.width).toBeLessThanOrEqual(rowBox!.x + rowBox!.width);
      }

      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        await page.evaluate(() => document.documentElement.clientWidth),
      );
    });
  }

  test("retains the dense table at normal desktop width", async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.goto("/knowledge-indexes");

    const table = page.getByRole("table", { name: "Knowledge indexes" });
    const row = table.locator('[data-slot="responsive-table-row"]');
    await expect(table.getByRole("columnheader", { name: "Name" })).toBeVisible();
    await expect(row.getByText("Source", { exact: true })).toBeHidden();
    await expect(row.getByRole("link", { name: "Open" })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      await page.evaluate(() => document.documentElement.clientWidth),
    );
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
