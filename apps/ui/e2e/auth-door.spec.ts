import { test, expect, type Page } from "@playwright/test";

/**
 * E2E coverage of the login-first door, the explicit signup path, and their
 * error/a11y contract. The backend is mocked with route interception so these run
 * hermetically in the ui-e2e job (no server, no real accounts).
 */

const AUTH_CONFIG = {
  mode: "full",
  password_auth_enabled: true,
  oauth_providers: ["google", "github"],
  signup_enabled: true,
  signup_email_confirm: true,
};

const NO_AUTH_CONFIG = {
  mode: "none",
  password_auth_enabled: false,
  oauth_providers: [],
  signup_enabled: false,
  signup_email_confirm: false,
};

async function mockAuthConfig(page: Page, config: Record<string, unknown> = AUTH_CONFIG) {
  await page.route("**/v1/auth/config", (route) => route.fulfill({ json: config }));
}

test("no-auth login reaches the application instead of a blank redirect loop", async ({ page }) => {
  await mockAuthConfig(page, NO_AUTH_CONFIG);
  await page.route("**/v1/auth/me", (route) =>
    route.fulfill({
      json: {
        id: "anonymous",
        email: "anonymous@localhost",
        display_name: "Anonymous",
        email_verified: true,
      },
    }),
  );

  await page.goto("/login");

  await expect(page).toHaveURL(/\/chats$/);
  await expect(page.getByRole("heading", { level: 1, name: "Chats" })).toBeVisible();
});

test.describe("Unified auth door", () => {
  test.beforeEach(async ({ page }) => {
    await mockAuthConfig(page);
  });

  test("phase 1 is login-first: SSO primary, email secondary, create-account link", async ({
    page,
  }) => {
    await page.goto("/login");
    await expect(page.getByRole("heading", { level: 1, name: "Log in" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Continue with Google" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Continue with GitHub" })).toBeVisible();
    await expect(page.getByLabel("Email")).toBeVisible();
    // No password field until the email is submitted.
    await expect(page.getByLabel("Password", { exact: true })).toHaveCount(0);
    // Explicit signup path, no silent fallback.
    await expect(page.getByRole("link", { name: "Create an account" })).toHaveAttribute(
      "href",
      "/signup",
    );
  });

  test("email continue moves to the password phase with the email locked in", async ({ page }) => {
    await page.goto("/login");
    await page.getByLabel("Email").fill("eli@acme.com");
    await page.getByRole("button", { name: "Continue with email" }).click();
    await expect(
      page.getByRole("heading", { level: 1, name: "Enter your password" }),
    ).toBeVisible();
    await expect(page.getByText("Logging in as")).toContainText("eli@acme.com");
    // Password field receives focus for keyboard users.
    await expect(page.getByLabel("Password", { exact: true })).toBeFocused();
    // Back returns to the email phase.
    await page.getByRole("button", { name: "Back" }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Log in" })).toBeVisible();
    await expect(page.getByLabel("Email")).toBeFocused();
  });

  test("password phase offers signup with the typed address carried over outside the URL", async ({
    page,
  }) => {
    await page.goto("/login");
    await page.getByLabel("Email").fill("newcomer@acme.com");
    await page.getByRole("button", { name: "Continue with email" }).click();
    await expect(
      page.getByRole("heading", { level: 1, name: "Enter your password" }),
    ).toBeVisible();
    // A new user who mistook login for signup is never stuck here.
    const signup = page.getByRole("link", { name: "Create an account" });
    await expect(signup).toHaveAttribute("href", "/signup");
    await signup.click();
    await expect(page).toHaveURL(/\/signup$/);
    await expect(
      page.getByRole("heading", { level: 1, name: "Create your account" }),
    ).toBeVisible();
    await expect(page.getByLabel("Email")).toHaveValue("newcomer@acme.com");
  });

  test("bad credentials render the calm generic error with a reset path", async ({ page }) => {
    await page.route("**/v1/auth/login", (route) =>
      route.fulfill({ status: 401, json: { error: "Invalid email or password" } }),
    );
    await page.goto("/login");
    await page.getByLabel("Email").fill("eli@acme.com");
    await page.getByRole("button", { name: "Continue with email" }).click();
    await page.getByLabel("Password", { exact: true }).fill("wrong-password");
    await page.getByRole("button", { name: "Continue", exact: true }).click();
    // Filter out Next's empty route announcer, which is also role=alert.
    const alert = page.getByRole("alert").filter({ hasText: /\S/ });
    await expect(alert).toBeVisible();
    // Calm generic copy only — no enumeration, no raw server message.
    await expect(alert).toContainText("Email or password doesn't match");
    // Never a dead end: the sentence ends in a prefilled reset link.
    const reset = alert.getByRole("link", { name: "reset your password" });
    await expect(reset).toHaveAttribute("href", "/forgot-password?email=eli%40acme.com");
    await reset.click();
    await expect(
      page.getByRole("heading", { level: 1, name: "Reset your password" }),
    ).toBeVisible();
    await expect(page.getByLabel("Email")).toHaveValue("eli@acme.com");
  });

  test("explicit signup: requirements gate, then unconditional check-your-email", async ({
    page,
  }) => {
    let registered = false;
    await page.route("**/v1/auth/register", (route) => {
      registered = true;
      return route.fulfill({ status: 200, json: { ok: true } });
    });
    await page.goto("/signup");
    await expect(
      page.getByRole("heading", { level: 1, name: "Create your account" }),
    ).toBeVisible();
    await page.getByLabel("Email").fill("new@acme.com");
    // Under-policy password: inline requirements stop the submit client-side.
    await page.getByLabel("Password", { exact: true }).fill("short1");
    await page.getByRole("button", { name: "Create account" }).click();
    expect(registered).toBe(false);
    // Meets policy: submits and lands on the generic confirmation state.
    await page.getByLabel("Password", { exact: true }).fill("longenoughpass1");
    await page.getByRole("button", { name: "Create account" }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Check your email" })).toBeVisible();
    // Full-sentence assertion: guards every word boundary in the copy — the
    // JSX pipeline once ate the space after the email, and a substring match
    // let it through.
    await expect(
      page.getByText(
        "If new@acme.com can be registered, we've sent a confirmation link. " +
          "Click it to verify your email and get started — the link signs you in.",
      ),
    ).toBeVisible();
    expect(registered).toBe(true);
  });

  test("signup with an existing email shows the identical confirmation", async ({ page }) => {
    // Confirm-mode backend answers 200 {ok:true} for taken addresses too;
    // the UI must not diverge.
    await page.route("**/v1/auth/register", (route) =>
      route.fulfill({ status: 200, json: { ok: true } }),
    );
    await page.goto("/signup");
    await page.getByLabel("Email").fill("taken@acme.com");
    await page.getByLabel("Password", { exact: true }).fill("longenoughpass1");
    await page.getByRole("button", { name: "Create account" }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Check your email" })).toBeVisible();
  });

  test("OAuth callback failure categories render friendly copy", async ({ page }) => {
    // Filter out Next's empty route announcer, which is also role=alert.
    const alert = () => page.getByRole("alert").filter({ hasText: /\S/ });
    await page.goto("/login?error=oauth_cancelled");
    await expect(alert()).toContainText("Sign-in was cancelled");
    await page.goto("/login?error=oauth_not_permitted");
    await expect(alert()).toContainText("can't be used to sign in here");
    // A permanent "already registered" refusal must name the way through, not
    // the transient "try again" copy (auth-flow dead-end audit).
    await page.goto("/login?error=oauth_account_exists");
    await expect(alert()).toContainText("already has an account");
    // Unknown categories fall back to the generic copy, never raw text.
    await page.goto("/login?error=<script>alert(1)</script>");
    await expect(alert()).toContainText("didn't complete");
  });

  test("credential failure points OAuth users back to their provider", async ({ page }) => {
    await page.route("**/v1/auth/login", (route) =>
      route.fulfill({ status: 401, json: { error: "Invalid email or password" } }),
    );
    await page.goto("/login");
    await page.getByLabel("Email").fill("googler@acme.com");
    await page.getByRole("button", { name: "Continue with email" }).click();
    await page.getByLabel("Password", { exact: true }).fill("wrong-password");
    await page.getByRole("button", { name: "Continue", exact: true }).click();
    const alert = page.getByRole("alert").filter({ hasText: /\S/ });
    // The OAuth-only reset trap: reset no-ops for these accounts, so the alert
    // must also offer the provider path. "Go back" returns to the SSO buttons.
    await expect(alert).toContainText("Signed up with Google or GitHub");
    await alert.getByRole("button", { name: "Go back" }).click();
    await expect(page.getByRole("button", { name: "Continue with Google" })).toBeVisible();
  });

  test("signup hides the password form when password auth is disabled", async ({ page }) => {
    await mockAuthConfig(page, { ...AUTH_CONFIG, password_auth_enabled: false });
    await page.goto("/signup");
    // OAuth stays; the password form (which would 403) must not render.
    await expect(page.getByRole("button", { name: "Continue with Google" })).toBeVisible();
    await expect(page.getByLabel("Password", { exact: true })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Create account" })).toHaveCount(0);
  });

  test("verify-email dead link lets the user request a fresh one in place", async ({ page }) => {
    let resendEmail: string | null = null;
    await page.route("**/v1/auth/resend-verification", (route, request) => {
      resendEmail = (request.postDataJSON() as { email: string }).email;
      return route.fulfill({ json: { ok: true } });
    });
    // No token, no email = the former dead end ("sign in to request…").
    await page.goto("/verify-email");
    await expect(
      page.getByRole("heading", { level: 1, name: "Verification failed" }),
    ).toBeVisible();
    // Now self-serve: enter an email and get a new link without leaving.
    await page.getByLabel(/Enter your email/).fill("stuck@acme.com");
    await page.getByRole("button", { name: /Resend/ }).click();
    await expect.poll(() => resendEmail).toBe("stuck@acme.com");
    await expect(page.getByText("we've sent a new verification link")).toBeVisible();
  });

  test("signup disabled hides the create-account link and closes /signup", async ({ page }) => {
    await mockAuthConfig(page, { ...AUTH_CONFIG, signup_enabled: false });
    await page.goto("/login");
    await expect(page.getByRole("link", { name: "Create an account" })).toHaveCount(0);
    await page.goto("/signup");
    await expect(page.getByRole("heading", { level: 1, name: "Signups are closed" })).toBeVisible();
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
    await page.getByLabel("New password").fill("password12345");
    await page.getByLabel("Confirm password").fill("password12345");
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
