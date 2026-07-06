import { test, expect, type Page } from "@playwright/test";

/**
 * E2E coverage of the unified "Log in or sign up" door and its error/a11y
 * contract. The backend is mocked with route interception so these run
 * hermetically in the ui-e2e job (no server, no real accounts).
 */

const AUTH_CONFIG = {
  mode: "full",
  password_auth_enabled: true,
  oauth_providers: ["google", "github"],
  signup_enabled: true,
};

async function mockAuthConfig(page: Page, config: Record<string, unknown> = AUTH_CONFIG) {
  await page.route("**/v1/auth/config", (route) =>
    route.fulfill({ json: config }),
  );
}

test.describe("Unified auth door", () => {
  test.beforeEach(async ({ page }) => {
    await mockAuthConfig(page);
  });

  test("phase 1 renders one door: SSO primary, email secondary, h1 heading", async ({
    page,
  }) => {
    await page.goto("/login");
    await expect(page.getByRole("heading", { level: 1, name: "Log in or sign up" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Continue with Google" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Continue with GitHub" })).toBeVisible();
    await expect(page.getByLabel("Email")).toBeVisible();
    // No password field until the email is submitted.
    await expect(page.getByLabel("Password", { exact: true })).toHaveCount(0);
  });

  test("email continue moves to the password phase with the email locked in", async ({
    page,
  }) => {
    await page.goto("/login");
    await page.getByLabel("Email").fill("eli@acme.com");
    await page.getByRole("button", { name: "Continue with email" }).click();
    await expect(
      page.getByRole("heading", { level: 1, name: "Enter your password" }),
    ).toBeVisible();
    await expect(page.getByText("Continuing as")).toContainText("eli@acme.com");
    // Password field receives focus for keyboard users.
    await expect(page.getByLabel("Password", { exact: true })).toBeFocused();
    // Back returns to the email phase.
    await page.getByRole("button", { name: "Back" }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Log in or sign up" })).toBeVisible();
    await expect(page.getByLabel("Email")).toBeFocused();
  });

  test("bad credentials render the generic error as an alert", async ({ page }) => {
    await page.route("**/v1/auth/login", (route) =>
      route.fulfill({ status: 401, json: { error: "Invalid email or password" } }),
    );
    await page.route("**/v1/auth/register", (route) =>
      route.fulfill({ status: 401, json: { error: "Registration failed" } }),
    );
    await page.goto("/login");
    await page.getByLabel("Email").fill("eli@acme.com");
    await page.getByRole("button", { name: "Continue with email" }).click();
    await page.getByLabel("Password", { exact: true }).fill("wrong-password");
    await page.getByRole("button", { name: "Continue", exact: true }).click();
    // Filter out Next's empty route announcer, which is also role=alert.
    const alert = page.getByRole("alert").filter({ hasText: /\S/ });
    await expect(alert).toBeVisible();
    // Generic copy only — never a raw server message or field-specific hint.
    await expect(alert).toContainText("Invalid email or password.");
  });

  test("unknown email with a valid password signs up through the same door", async ({
    page,
  }) => {
    await page.route("**/v1/auth/login", (route) =>
      route.fulfill({ status: 401, json: { error: "Invalid email or password" } }),
    );
    let registered = false;
    await page.route("**/v1/auth/register", (route) => {
      registered = true;
      return route.fulfill({
        status: 201,
        json: { access_token: "t", token_type: "Bearer", expires_in: 900 },
      });
    });
    await page.route("**/v1/auth/me", (route) =>
      route.fulfill({
        json: { id: "u1", email: "new@acme.com", name: "New", roles: ["user"] },
      }),
    );
    await page.goto("/login");
    await page.getByLabel("Email").fill("new@acme.com");
    await page.getByRole("button", { name: "Continue with email" }).click();
    await page.getByLabel("Password", { exact: true }).fill("a-strong-password");
    await page.getByRole("button", { name: "Continue", exact: true }).click();
    await expect.poll(() => registered).toBe(true);
  });

  test("OAuth callback failure categories render friendly copy", async ({ page }) => {
    // Filter out Next's empty route announcer, which is also role=alert.
    const alert = () => page.getByRole("alert").filter({ hasText: /\S/ });
    await page.goto("/login?error=oauth_cancelled");
    await expect(alert()).toContainText("Sign-in was cancelled");
    await page.goto("/login?error=oauth_not_permitted");
    await expect(alert()).toContainText("can't be used to sign in here");
    // Unknown categories fall back to the generic copy, never raw text.
    await page.goto("/login?error=<script>alert(1)</script>");
    await expect(alert()).toContainText("didn't complete");
  });

  test("signup disabled turns the door into welcome-back", async ({ page }) => {
    await mockAuthConfig(page, { ...AUTH_CONFIG, signup_enabled: false });
    await page.goto("/login");
    await expect(page.getByRole("heading", { level: 1, name: "Welcome back" })).toBeVisible();
  });
});

test.describe("Password recovery pages", () => {
  test.beforeEach(async ({ page }) => {
    await mockAuthConfig(page);
  });

  test("forgot-password confirms without revealing account existence", async ({ page }) => {
    await page.route("**/v1/auth/forgot-password", (route) =>
      route.fulfill({ json: { ok: true } }),
    );
    await page.goto("/forgot-password");
    await expect(
      page.getByRole("heading", { level: 1, name: "Reset your password" }),
    ).toBeVisible();
    await page.getByLabel("Email").fill("ghost@acme.com");
    await page.getByRole("button", { name: "Send reset link" }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Check your inbox" })).toBeVisible();
    await expect(page.getByText("If an account exists for")).toBeVisible();
  });

  test("reset-password with a missing token shows the invalid-link state", async ({ page }) => {
    await page.goto("/reset-password");
    await expect(page.getByRole("heading", { level: 1, name: "Invalid reset link" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Request a new link" })).toBeVisible();
  });

  test("reset-password with an expired token swaps to the request-a-new-link state", async ({
    page,
  }) => {
    await page.route("**/v1/auth/reset-password", (route) =>
      route.fulfill({ status: 400, json: { error: "Invalid or expired reset token" } }),
    );
    await page.goto("/reset-password?token=stale");
    await page.getByLabel("New password").fill("password123");
    await page.getByLabel("Confirm password").fill("password123");
    await page.getByRole("button", { name: "Update password" }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Invalid reset link" })).toBeVisible();
  });

  test("verify-email pending state offers resend with a cooldown", async ({ page }) => {
    let resendCalls = 0;
    await page.route("**/v1/auth/resend-verification", (route) => {
      resendCalls += 1;
      return route.fulfill({ json: { ok: true } });
    });
    await page.goto("/verify-email?email=eli%40acme.com");
    await expect(page.getByRole("heading", { level: 1, name: "Verify your email" })).toBeVisible();
    await expect(page.getByText("eli@acme.com")).toBeVisible();
    const resend = page.getByRole("button", { name: /Resend/ });
    await resend.click();
    await expect.poll(() => resendCalls).toBe(1);
    // Cooldown arms and the button locks — matches the server send budget.
    await expect(page.getByRole("button", { name: /Resend link · 0:/ })).toBeDisabled();
    await expect(page.getByText("Use a different email")).toBeVisible();
  });
});
