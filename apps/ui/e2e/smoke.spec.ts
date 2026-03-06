import { test, expect } from "@playwright/test";

/**
 * Smoke tests for deployed environments (e.g., dev.everruns.com).
 *
 * Run:
 *   doppler run -- npx playwright test --project smoke
 *
 * Required env:
 *   SMOKE_BASE_URL (or defaults to https://dev.everruns.com)
 *   TEST_USER_1_PASSWORD (for admin/full auth modes with password login)
 *
 * Works with auth modes: none, admin, full, external
 */

const TEST_EMAIL = process.env.TEST_USER_1_EMAIL || "aliceeverruns@gmail.com";

// Fetch auth config once and cache
let authConfig: { mode: string; password_auth_enabled: boolean } | null = null;

async function getAuthConfig(
  request: import("@playwright/test").APIRequestContext,
): Promise<typeof authConfig> {
  if (authConfig) return authConfig;
  const resp = await request.get("/api/v1/auth/config");
  expect(resp.ok()).toBeTruthy();
  authConfig = await resp.json();
  return authConfig;
}

test.describe("Smoke: health", () => {
  test("health endpoint returns ok", async ({ request }) => {
    const resp = await request.get("/health");
    expect(resp.ok()).toBeTruthy();
    const body = await resp.json();
    expect(body.status).toBe("ok");
  });

  test("auth config endpoint returns valid config", async ({ request }) => {
    const resp = await request.get("/api/v1/auth/config");
    expect(resp.ok()).toBeTruthy();
    const body = await resp.json();
    expect(body.mode).toBeTruthy();
    expect(["none", "admin", "full", "external"]).toContain(body.mode);
  });
});

test.describe("Smoke: login flow", () => {
  test("login page loads", async ({ page }) => {
    await page.goto("/login");
    // Login page should show the Everruns branding regardless of auth mode
    // In "none" mode it may redirect to dashboard
    await page.waitForLoadState("networkidle");
    const url = page.url();
    expect(url).toMatch(/\/(login|dashboard)/);
  });

  test("password login and dashboard", async ({ page, request }) => {
    const config = await getAuthConfig(request);

    // Skip if password auth is not enabled
    test.skip(
      !config!.password_auth_enabled,
      "Password auth not enabled (auth mode: " + config!.mode + ")",
    );

    await page.goto("/login");
    await expect(page.getByText("Welcome back")).toBeVisible({ timeout: 15000 });

    await page.getByLabel("Email").fill(TEST_EMAIL);
    await page.getByLabel("Password").fill(process.env.TEST_USER_1_PASSWORD!);
    await page.getByRole("button", { name: "Sign in" }).click();

    await expect(page).toHaveURL(/\/dashboard/, { timeout: 15000 });
    await expect(page.getByRole("heading", { name: "Dashboard" })).toBeVisible({ timeout: 10000 });
  });

  test("navigate to agents page after login", async ({ page, request }) => {
    const config = await getAuthConfig(request);
    test.skip(!config!.password_auth_enabled, "Password auth not enabled");

    await loginWithPassword(page);

    await page.getByRole("link", { name: "Agents" }).first().click();
    await expect(page).toHaveURL(/\/agents/, { timeout: 10000 });
    await expect(page.getByRole("heading", { name: "Agents" })).toBeVisible({ timeout: 10000 });
  });

  test("navigate to sessions page after login", async ({ page, request }) => {
    const config = await getAuthConfig(request);
    test.skip(!config!.password_auth_enabled, "Password auth not enabled");

    await loginWithPassword(page);

    await page.getByRole("link", { name: "Sessions" }).first().click();
    await expect(page).toHaveURL(/\/sessions/, { timeout: 10000 });
    await expect(page.getByRole("heading", { name: "Sessions" })).toBeVisible({ timeout: 10000 });
  });

  test("logout after login", async ({ page, request }) => {
    const config = await getAuthConfig(request);
    test.skip(!config!.password_auth_enabled, "Password auth not enabled");

    await loginWithPassword(page);

    const userName = process.env.TEST_USER_1_NAME || "Alice Everruns";
    await page.getByText(userName).click();
    await page.getByRole("menuitem", { name: /sign out/i }).click();

    await expect(page).toHaveURL(/\/login/, { timeout: 10000 });
  });
});

/** Helper: password login and wait for dashboard */
async function loginWithPassword(page: import("@playwright/test").Page) {
  await page.goto("/login");
  await expect(page.getByText("Welcome back")).toBeVisible({ timeout: 15000 });
  await page.getByLabel("Email").fill(TEST_EMAIL);
  await page.getByLabel("Password").fill(process.env.TEST_USER_1_PASSWORD!);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).toHaveURL(/\/dashboard/, { timeout: 15000 });
  await expect(page.getByRole("heading", { name: "Dashboard" })).toBeVisible({ timeout: 10000 });
}
