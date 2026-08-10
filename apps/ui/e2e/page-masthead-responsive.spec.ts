import { expect, test, type Page } from "@playwright/test";

const DEFAULT_ORG_ID = "org_00000000000000000000000000000001";
const AGENT_ID = "agent_019fd9b43fa37512b8f25226b21c2c8b";
const HARNESS_ID = "harness_019fd9b43fa37512b8f25226b21c2c8b";

async function mockAgentDetailApi(page: Page) {
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
        evals: true,
        app_budgets: false,
        agent_versions: true,
        voice: false,
        agent_delegation: false,
        observers: true,
        public_chat: false,
      };
    } else if (pathname.endsWith("/switch-org")) {
      json = { success: true, org_id: DEFAULT_ORG_ID };
    } else if (pathname === `/api/v1/agents/${AGENT_ID}`) {
      json = {
        id: AGENT_ID,
        name: "jokes-agent",
        harness_id: "harness_test",
        display_name: "Jokes Agent",
        description:
          "A cheerful agent with a deliberately longer localized description for responsive layout testing.",
        system_prompt: "Tell a joke.",
        default_model_id: null,
        tags: [],
        capabilities: [],
        status: "active",
        session_count: 12,
        app_count: 3,
        created_at: "2026-08-01T00:00:00Z",
        updated_at: "2026-08-01T00:00:00Z",
        archived_at: null,
        deleted_at: null,
      };
    } else if (pathname === `/api/v1/agents/${AGENT_ID}/stats`) {
      json = {
        sessions: 12,
        messages: 24,
        input_tokens: 1200,
        output_tokens: 600,
      };
    } else if (pathname === `/api/v1/harnesses/${HARNESS_ID}`) {
      json = {
        id: HARNESS_ID,
        name: "responsive-harness",
        display_name: "Responsive Harness",
        description: "A representative detail page using the standard masthead action cluster.",
        system_prompt: "Help the user.",
        default_model_id: null,
        parent_harness_id: null,
        tags: [],
        capabilities: [],
        initial_files: [],
        is_built_in: false,
        status: "active",
        session_count: 4,
        app_count: 2,
        created_at: "2026-08-01T00:00:00Z",
        updated_at: "2026-08-01T00:00:00Z",
        archived_at: null,
        deleted_at: null,
      };
    } else if (pathname === `/api/v1/harnesses/${HARNESS_ID}/stats`) {
      json = { sessions: 4, messages: 8, input_tokens: 400, output_tokens: 200 };
    } else if (pathname === "/api/v1/sessions") {
      json = { data: [], has_more: false, total: 0, offset: 0, limit: 10 };
    } else if (
      pathname === "/api/v1/capabilities" ||
      pathname === "/api/v1/models" ||
      pathname === "/api/v1/harnesses"
    ) {
      json = { data: [], has_more: false, total: 0 };
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

    await route.fulfill({ json });
  });
}

test.describe("Page masthead responsive layout", () => {
  test.beforeEach(async ({ context, page, baseURL }) => {
    const origin = new URL(baseURL ?? "http://localhost:9100");
    await context.addCookies([
      { name: "access_token", value: "e2e-only", domain: origin.hostname, path: "/" },
    ]);
    await mockAgentDetailApi(page);
  });

  test("uses overflow-only actions at mobile width", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(`/agents/${AGENT_ID}`);

    const title = page.getByRole("heading", { name: "Jokes Agent" });
    const masthead = title.locator("xpath=ancestor::div[@data-slot='page-masthead'][1]");
    const identity = masthead.locator('[data-slot="page-masthead-identity"]');
    const icon = identity.locator('[data-slot="icon-tile"]');
    const actions = masthead.locator('[data-slot="page-masthead-compact-actions"]');
    const moreActions = page.getByRole("button", { name: "More actions" });
    await expect(title).toBeVisible();
    await expect(page.getByRole("button", { name: "Open navigation" })).toBeVisible();
    await expect(actions).toHaveCount(1);
    await expect(moreActions).toBeVisible();
    await expect(page.getByRole("button", { name: "Copy", exact: true })).toBeHidden();
    await expect(page.getByRole("button", { name: "Observe this agent" })).toBeHidden();

    await page.getByRole("button", { name: "Open navigation" }).click();
    await expect(page.locator('[data-slot="drawer-content"]')).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator('[data-slot="drawer-content"]')).toBeHidden();

    await moreActions.click({ trial: true });
    await moreActions.click();
    await expect(page.getByRole("menuitem", { name: "Copy" })).toBeVisible();
    await expect(page.getByRole("menuitem", { name: "Export" })).toBeVisible();
    await expect(page.getByRole("menuitem", { name: "Edit" })).toBeVisible();
    await expect(page.getByRole("menuitem", { name: "Create app" })).toBeVisible();
    await expect(page.getByRole("menuitem", { name: "Observe this agent" })).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(moreActions).toBeFocused();
    await moreActions.press("Enter");
    await expect(page.getByRole("menuitem", { name: "Copy" })).toBeVisible();
    await expect(page.getByRole("menuitem", { name: "Observe this agent" })).toBeVisible();
    await page.keyboard.press("Escape");

    const mastheadBox = await masthead.boundingBox();
    const identityBox = await identity.boundingBox();
    const iconBox = await icon.boundingBox();
    const actionsBox = await actions.boundingBox();
    const titleBox = await title.boundingBox();

    expect(mastheadBox).not.toBeNull();
    expect(identityBox).not.toBeNull();
    expect(iconBox).not.toBeNull();
    expect(actionsBox).not.toBeNull();
    expect(titleBox).not.toBeNull();
    expect(Math.abs(actionsBox!.y - iconBox!.y)).toBeLessThanOrEqual(1);
    expect(Math.max(actionsBox!.y + actionsBox!.height, iconBox!.y + iconBox!.height)).toBeLessThan(
      titleBox!.y,
    );
    expect(mastheadBox!.width).toBeGreaterThanOrEqual(300);
    expect(identityBox!.width).toBeGreaterThanOrEqual(Math.min(320, mastheadBox!.width));
    expect(actionsBox!.x + actionsBox!.width).toBeLessThanOrEqual(
      mastheadBox!.x + mastheadBox!.width,
    );
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      await page.evaluate(() => document.documentElement.clientWidth),
    );
  });

  for (const viewport of [
    { name: "compact desktop", width: 1024, height: 900 },
    { name: "tablet", width: 768, height: 900 },
  ]) {
    test(`places prioritized actions above identity at ${viewport.name} width`, async ({ page }) => {
      await page.setViewportSize({ width: viewport.width, height: viewport.height });
      await page.goto(`/agents/${AGENT_ID}`);

      const title = page.getByRole("heading", { name: "Jokes Agent" });
      const masthead = title.locator("xpath=ancestor::div[@data-slot='page-masthead'][1]");
      const icon = masthead.locator('[data-slot="icon-tile"]');
      const actions = page
        .getByRole("button", { name: "New session" })
        .locator("xpath=ancestor::div[@data-slot='page-masthead-compact-actions'][1]");
      const actionStrip = masthead.locator('[data-slot="page-masthead-compact-action-strip"]');

      await expect(page.getByRole("button", { name: "New session" })).toBeVisible();
      await expect(page.getByRole("button", { name: "More actions" })).toBeVisible();
      await expect(page.getByRole("button", { name: "Copy", exact: true })).toBeVisible();
      await expect(page.getByRole("button", { name: "Export", exact: true })).toBeVisible();
      await expect(page.getByRole("link", { name: "Edit" })).toBeVisible();
      await expect(page.getByRole("link", { name: "Create app" })).toBeVisible();
      await expect(page.getByRole("button", { name: "Observe this agent" })).toBeHidden();

      await page.getByRole("button", { name: "More actions" }).click();
      await expect(page.getByRole("menuitem", { name: "Observe this agent" })).toBeVisible();
      await expect(page.getByRole("menuitem", { name: "Copy" })).toHaveCount(0);

      const mastheadBox = await masthead.boundingBox();
      const iconBox = await icon.boundingBox();
      const actionsBox = await actions.boundingBox();
      const stripBox = await actionStrip.boundingBox();
      expect(mastheadBox).not.toBeNull();
      expect(iconBox).not.toBeNull();
      expect(actionsBox).not.toBeNull();
      expect(stripBox).not.toBeNull();
      expect(actionsBox!.y + actionsBox!.height).toBeLessThanOrEqual(iconBox!.y);
      expect(stripBox!.width).toBeLessThanOrEqual(mastheadBox!.width);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        await page.evaluate(() => document.documentElement.clientWidth),
      );
    });
  }

  test("keeps actions aligned beside identity content at wide desktop width", async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.goto(`/agents/${AGENT_ID}`);

    const title = page.getByRole("heading", { name: "Jokes Agent" });
    const actions = page
      .getByRole("button", { name: "New session" })
      .locator("xpath=ancestor::div[@data-slot='page-masthead-actions'][1]");
    const titleBox = await title.boundingBox();
    const actionsBox = await actions.boundingBox();

    expect(titleBox).not.toBeNull();
    expect(actionsBox).not.toBeNull();
    expect(actionsBox!.y).toBeLessThan(titleBox!.y + titleBox!.height);
    await expect(page.getByRole("button", { name: "Open navigation" })).toBeHidden();
    await expect(page.getByRole("button", { name: "More actions" })).toBeHidden();
    await expect(page.getByRole("button", { name: "Observe this agent" })).toBeVisible();
  });

  test("keeps a representative standard-action consumer contained on mobile", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(`/harnesses/${HARNESS_ID}`);

    const title = page.getByRole("heading", { name: "Responsive Harness" });
    const masthead = title.locator("xpath=ancestor::div[@data-slot='page-masthead'][1]");
    const edit = page.getByRole("link", { name: "Edit" });

    await expect(title).toBeVisible();
    await expect(page.getByRole("link", { name: "Create app" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Copy", exact: true })).toBeVisible();
    await expect(edit).toBeVisible();
    await expect(page.getByRole("button", { name: "Archive" })).toBeVisible();
    await edit.click({ trial: true });

    const mastheadBox = await masthead.boundingBox();
    expect(mastheadBox).not.toBeNull();
    expect(mastheadBox!.x + mastheadBox!.width).toBeLessThanOrEqual(
      await page.evaluate(() => document.documentElement.clientWidth),
    );
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      await page.evaluate(() => document.documentElement.clientWidth),
    );
  });
});
